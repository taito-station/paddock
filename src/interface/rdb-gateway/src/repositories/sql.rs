//! stats クエリ共通の SQL 片ヘルパ。

use std::collections::HashMap;

use paddock_use_case::repository::GroupStat;
use sqlx::PgPool;

/// 成績集計の SELECT 本体（COUNT/SUM(CASE...) と `results INNER JOIN races`）。
/// stats 系クエリ（jockey/trainer/course）で共通。呼び出し側が ` WHERE ...` を後続させる。
pub(crate) const STATS_AGG_SELECT: &str = r#"
    SELECT
        COUNT(*) AS starts,
        COALESCE(SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END), 0) AS wins,
        COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END), 0) AS places,
        COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END), 0) AS shows
    FROM results
    INNER JOIN races ON races.race_id = results.race_id
"#;

/// LIKE のワイルドカード（`%` / `_`）とエスケープ文字（`\`）をリテラル化する。
/// `query` 中にこれらが混じっても任意一致せず、入力文字そのものとして検索する。
/// `LIKE '%' || $n || '%' ESCAPE '\'` 形式のクエリにバインドする値を作る共通ヘルパ
/// （`find_matching_names` / `pad_prediction` の馬名検索で共有, #145）。
pub(crate) fn escape_like(query: &str) -> String {
    query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// 芝ダ別グループの (DB キー, 日本語ラベル)。
pub(crate) const SURFACE_KEYS: &[(&str, &str)] = &[("turf", "芝"), ("dirt", "ダート")];

/// 枠順別グループの (ラベル, 述語)。`gate_num >= 9`（フルゲート大外）はどのグループにも入らない
/// 既存仕様を踏襲する（変更は挙動を変えるため本リファクタの対象外）。
pub(crate) const GATE_GROUPS: &[(&str, &str)] = &[
    ("Inner (1-3)", "results.gate_num BETWEEN 1 AND 3"),
    ("Middle (4-6)", "results.gate_num BETWEEN 4 AND 6"),
    ("Outer (7-8)", "results.gate_num BETWEEN 7 AND 8"),
];

/// `(label, predicate)` の並びから `CASE WHEN <pred> THEN '<label>' ... END` を組み立てる。
/// GROUP BY のキー式に使う。`label`・`predicate` はいずれも呼び出し側の **code 定数**
/// （`GATE_GROUPS` 等）前提で、ユーザー入力を渡さない（`AssertSqlSafe` の契約・二重防御, #358）。
pub(crate) fn case_from_preds(arms: &[(&str, &str)]) -> String {
    let mut s = String::from("CASE");
    for (label, pred) in arms {
        s.push_str(&format!(" WHEN {pred} THEN '{label}'"));
    }
    s.push_str(" END");
    s
}

/// `(label, predicate)` の述語部だけを `(<pred>) OR (<pred>) ...` で OR 連結する。
/// 対象セルの行だけに絞る WHERE 片に使う（非セル行＝枠9+・NULL 馬場を除外）。
/// `predicate` は code 定数前提（`case_from_preds` と同じ SQL 安全契約, #358）。
pub(crate) fn or_from_preds(arms: &[(&str, &str)]) -> String {
    arms.iter()
        .map(|(_, pred)| format!("({pred})"))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// 集計結果の 4-tuple `(starts, wins, places, shows)` を [`GroupStat`] に詰める。
pub(crate) fn group_stat_from_row(label: &str, row: (i64, i64, i64, i64)) -> GroupStat {
    GroupStat {
        label: label.to_string(),
        starts: row.0 as u32,
        wins: row.1 as u32,
        places: row.2 as u32,
        shows: row.3 as u32,
    }
}

/// `[STATS_AGG_SELECT] WHERE ...` のクエリに `binds`（$1..）と任意の `cutoff`（末尾）を
/// 順にバインドして 1 行の集計 tuple を取得する。`cutoff` は `date_lt_pred` のプレースホルダに対応。
async fn fetch_agg(
    pool: &PgPool,
    query: String,
    binds: &[&str],
    cutoff: Option<&str>,
) -> crate::error::Result<(i64, i64, i64, i64)> {
    let mut q = sqlx::query_as(sqlx::AssertSqlSafe(query));
    for b in binds {
        q = q.bind(*b);
    }
    if let Some(d) = cutoff {
        q = q.bind(d);
    }
    Ok(q.fetch_one(pool).await?)
}

/// `results.<column> = value` を母数にした成績集計を (overall, 芝ダ別, 枠順別) で返す。
/// `jockey_stats` / `trainer_stats` が共有する（列名と Row 型以外は同型, #85）。
///
/// `column` は SQL に直接埋め込むため既知リテラル（`"jockey"` / `"trainer"`）のみを許す
/// （`delete_absent_horse_nums` と同じ二重防御）。`value` と `cutoff` はプレースホルダで
/// バインドする（SQL インジェクション安全）。`as_of = None`（cutoff なし）の結果は従来と一致。
pub(crate) async fn entity_stats(
    pool: &PgPool,
    column: &str,
    value: &str,
    cutoff: Option<&str>,
) -> crate::error::Result<(GroupStat, Vec<GroupStat>, Vec<GroupStat>)> {
    debug_assert!(
        matches!(column, "jockey" | "trainer"),
        "column must be a known literal, got {column:?}"
    );

    let overall_q = format!(
        "{STATS_AGG_SELECT} WHERE results.{column} = $1 \
         AND results.finishing_position IS NOT NULL {date}",
        date = date_lt_pred(cutoff, "$2"),
    );
    let overall = group_stat_from_row("全体", fetch_agg(pool, overall_q, &[value], cutoff).await?);

    let mut by_surface = Vec::with_capacity(SURFACE_KEYS.len());
    for (key, label) in SURFACE_KEYS {
        let q = format!(
            "{STATS_AGG_SELECT} WHERE results.{column} = $1 \
             AND results.finishing_position IS NOT NULL AND races.surface = $2 {date}",
            date = date_lt_pred(cutoff, "$3"),
        );
        let row = fetch_agg(pool, q, &[value, key], cutoff).await?;
        by_surface.push(group_stat_from_row(label, row));
    }

    let mut by_gate_group = Vec::with_capacity(GATE_GROUPS.len());
    for (label, predicate) in GATE_GROUPS {
        let q = format!(
            "{STATS_AGG_SELECT} WHERE results.{column} = $1 \
             AND results.finishing_position IS NOT NULL AND {predicate} {date}",
            date = date_lt_pred(cutoff, "$2"),
        );
        let row = fetch_agg(pool, q, &[value], cutoff).await?;
        by_gate_group.push(group_stat_from_row(label, row));
    }

    Ok((overall, by_surface, by_gate_group))
}

/// `STATS_AGG_SELECT` に `results.<column>` を加え、`WHERE results.<column> = ANY($1) AND <tail>`
/// と `GROUP BY results.<column>` で全エンティティ一括集計し、`entity 値 -> 4-tuple` を返す
/// （#196。`entity_stats` の per-item を全エンティティバッチ化したもの）。`tail` は WHERE 後半
/// （`finishing_position IS NOT NULL ...` 以降、`date_lt_pred` まで）で、`extra_binds` は `$2..`、
/// cutoff は末尾に積む。
async fn fetch_agg_grouped(
    pool: &PgPool,
    column: &str,
    values: &[&str],
    tail: &str,
    extra_binds: &[&str],
    cutoff: Option<&str>,
) -> crate::error::Result<HashMap<String, (i64, i64, i64, i64)>> {
    let query_str = format!(
        r#"
        SELECT
            results.{column} AS entity,
            COUNT(*) AS starts,
            COALESCE(SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END), 0) AS wins,
            COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END), 0) AS places,
            COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END), 0) AS shows
        FROM results
        INNER JOIN races ON races.race_id = results.race_id
        WHERE results.{column} = ANY($1)
          {tail}
        GROUP BY results.{column}
        "#,
    );
    let mut q = sqlx::query_as::<_, (String, i64, i64, i64, i64)>(sqlx::AssertSqlSafe(query_str))
        .bind(values);
    for b in extra_binds {
        q = q.bind(*b);
    }
    if let Some(d) = cutoff {
        q = q.bind(d);
    }
    let rows = q.fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|(v, s, w, p, sh)| (v, (s, w, p, sh)))
        .collect())
}

/// グルーピング結果から `value` の `label` 行を取り出す。無ければ全ゼロを合成（per-item の
/// COUNT(*)=0 と同一 `GroupStat`）。
fn grouped_stat(
    map: &HashMap<String, (i64, i64, i64, i64)>,
    value: &str,
    label: &str,
) -> GroupStat {
    group_stat_from_row(label, map.get(value).copied().unwrap_or((0, 0, 0, 0)))
}

/// `entity_stats` の per-item を全エンティティバッチ化（#196）。`values` の各値をキーに
/// `(overall, by_surface, by_gate_group)` を返す。WHERE 述語・集計式・ラベル順は per-item と完全同値で、
/// 結果に現れないエンティティは全ゼロを合成する。`as_of = None` の結果は従来と一致。
pub(crate) async fn entity_stats_batch(
    pool: &PgPool,
    column: &str,
    values: &[&str],
    cutoff: Option<&str>,
) -> crate::error::Result<HashMap<String, (GroupStat, Vec<GroupStat>, Vec<GroupStat>)>> {
    debug_assert!(
        matches!(column, "jockey" | "trainer"),
        "column must be a known literal, got {column:?}"
    );
    if values.is_empty() {
        return Ok(HashMap::new());
    }

    let overall_map = fetch_agg_grouped(
        pool,
        column,
        values,
        &format!(
            "AND results.finishing_position IS NOT NULL {date}",
            date = date_lt_pred(cutoff, "$2"),
        ),
        &[],
        cutoff,
    )
    .await?;

    let mut surface_maps = Vec::with_capacity(SURFACE_KEYS.len());
    for (key, label) in SURFACE_KEYS {
        let map = fetch_agg_grouped(
            pool,
            column,
            values,
            &format!(
                "AND results.finishing_position IS NOT NULL AND races.surface = $2 {date}",
                date = date_lt_pred(cutoff, "$3"),
            ),
            &[key],
            cutoff,
        )
        .await?;
        surface_maps.push((*label, map));
    }

    let mut gate_maps = Vec::with_capacity(GATE_GROUPS.len());
    for (label, predicate) in GATE_GROUPS {
        let map = fetch_agg_grouped(
            pool,
            column,
            values,
            &format!(
                "AND results.finishing_position IS NOT NULL AND {predicate} {date}",
                date = date_lt_pred(cutoff, "$2"),
            ),
            &[],
            cutoff,
        )
        .await?;
        gate_maps.push((*label, map));
    }

    let mut out = HashMap::with_capacity(values.len());
    for value in values {
        if out.contains_key(*value) {
            continue;
        }
        let overall = grouped_stat(&overall_map, value, "全体");
        let by_surface = surface_maps
            .iter()
            .map(|(label, m)| grouped_stat(m, value, label))
            .collect();
        let by_gate_group = gate_maps
            .iter()
            .map(|(label, m)| grouped_stat(m, value, label))
            .collect();
        out.insert(value.to_string(), (overall, by_surface, by_gate_group));
    }
    Ok(out)
}

/// `as_of` カットオフ用の `AND races.date < <placeholder>` 述語片を返す。
///
/// `cutoff` が `None`（全期間集計）なら空文字。日付値は呼び出し側がプレースホルダで
/// バインドするため、ここでは文字列連結に値を含めない（SQL インジェクション安全）。
pub(crate) fn date_lt_pred(cutoff: Option<&str>, placeholder: &str) -> String {
    if cutoff.is_some() {
        format!("AND races.date < {placeholder}")
    } else {
        String::new()
    }
}

/// 再取り込み時に、今回の出走集合 `horse_nums` に含まれない馬番の行だけを `table` から削除する。
///
/// 破壊的な全消し（`DELETE ... WHERE race_id = ?`）の代替。出走中の馬は `ON CONFLICT` 更新で
/// 残しつつ、取消・除外で今回いなくなった馬番の残骸だけを掃除する。`horse_nums` が空のときは
/// 何もしない（劣化パースで全行を消さないための防御）。
///
/// `table` はリテラル（`"results"` / `"horse_entries"`）のみを渡す前提で `format!` に埋め込む。
/// race_id・馬番は必ずプレースホルダでバインドする（SQL インジェクション安全）。
pub(crate) async fn delete_absent_horse_nums(
    conn: &mut sqlx::PgConnection,
    table: &str,
    race_id: &str,
    horse_nums: &[i64],
) -> Result<(), sqlx::Error> {
    // `table` は SQL に直接埋め込むため、既知のリテラルのみ許す（呼び出し側契約の二重防御）。
    debug_assert!(
        matches!(table, "results" | "horse_entries"),
        "table must be a known literal, got {table:?}"
    );
    if horse_nums.is_empty() {
        return Ok(());
    }
    // race_id = $1、馬番は $2.. と番号付きで並べる（Postgres は番号付きプレースホルダ）。
    let placeholders = (0..horse_nums.len())
        .map(|i| format!("${}", i + 2))
        .collect::<Vec<_>>()
        .join(", ");
    let sql =
        format!("DELETE FROM {table} WHERE race_id = $1 AND horse_num NOT IN ({placeholders})");
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(race_id);
    for n in horse_nums {
        q = q.bind(n);
    }
    q.execute(conn).await?;
    Ok(())
}
