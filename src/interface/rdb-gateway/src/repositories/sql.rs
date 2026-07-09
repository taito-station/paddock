//! stats クエリ共通の SQL 片ヘルパ。

use std::collections::HashMap;

use paddock_use_case::repository::GroupStat;
use sqlx::PgPool;

/// 成績集計の集計式（starts/wins/places/shows）。SELECT リストに埋め込む共通片で、
/// キー列を前置するグループ集計（`fetch_agg_grouped` / `conditional_gate_stats`）が共有し、
/// ベース率定義のドリフトを防ぐ（#358）。`STATS_AGG_SELECT` の集計部と同一（const 連結が
/// できないためリテラルは 2 箇所に残るが、変更時は必ず両方を揃える）。
pub(crate) const STATS_AGG_EXPRS: &str = "COUNT(*) AS starts,
        COALESCE(SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END), 0) AS wins,
        COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END), 0) AS places,
        COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END), 0) AS shows";

/// 成績集計の SELECT 本体（`STATS_AGG_EXPRS` の集計式と `results INNER JOIN races`）。
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
        // ラベルは SQL 文字列リテラルに素で埋める（code 定数前提）。単一引用符が混じると
        // 破損/注入し得るため契約を機械化（`delete_absent_horse_nums` の debug_assert と同型）。
        debug_assert!(
            !label.contains('\''),
            "label must not contain a single quote: {label:?}"
        );
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

/// `(label, lo, hi)` の並びから `CASE WHEN <col> BETWEEN lo AND hi THEN '<label>' ... END` を組む。
/// 数値帯（頭数帯など）のキー化用。`case_from_preds` の BETWEEN 版で、ラベルの SQL リテラル
/// 埋め込み契約（単一引用符禁止の `debug_assert`）を同じ 1 箇所に集約する（#358）。
/// `col` は呼び出し側の code 定数（列式）前提でユーザー入力を渡さない。
pub(crate) fn case_from_bands(col: &str, bands: &[(&str, u32, u32)]) -> String {
    let mut s = String::from("CASE");
    for (label, lo, hi) in bands {
        debug_assert!(
            !label.contains('\''),
            "band label must not contain a single quote: {label:?}"
        );
        s.push_str(&format!(" WHEN {col} BETWEEN {lo} AND {hi} THEN '{label}'"));
    }
    s.push_str(" END");
    s
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
            {STATS_AGG_EXPRS}
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

/// 距離帯（band）の (ラベル, 述語)。相性 factor（#350）の jockey_distance が使う。ラベルは
/// build_factors の `distance_band_label` と、述語は horse_stats の `group_by_distance_band` と
/// 完全一致させる（跨るラベルが factor の照合キーになるため drift は factor を静かに欠落させる）。
pub(crate) const DISTANCE_BAND_PREDS: &[(&str, &str)] = &[
    ("〜1400m", "races.distance <= 1400"),
    ("1500〜1800m", "races.distance BETWEEN 1500 AND 1800"),
    ("1900〜2200m", "races.distance BETWEEN 1900 AND 2200"),
    ("2300m〜", "races.distance >= 2300"),
];

/// `dynamic_group_stats(_batch)` の `group_col` が **code 定数の列参照 or CASE 式** であることを
/// 機械的に担保する（#350・二重防御の対称化。`entity_col` の `matches!` guard と揃える）。現行の
/// 呼び出しは `races.venue` / `results.jockey` / `case_from_preds(..)` 生成の `CASE ... END` のみで、
/// いずれも単一引用符でリテラルを破らない code 定数。将来の呼び出し追加で外部入力が group_col に
/// 混じる回帰を debug ビルドで即検知する（`case_from_preds` の label 側 debug_assert と同型の契約）。
///
/// **契約**: CASE 式を渡すときは必ず `case_from_preds` / `case_from_bands` 経由で生成すること
/// （label 側の単一引用符が debug_assert 済みで注入不可）。本 guard は shape（`CASE` 始まり）しか
/// 見ないため、`case_from_preds` を経由しない生の CASE 文字列（predicate 側に外部入力を含みうる）を
/// 渡すと素通しになる。列参照は既知 2 種の厳密一致に限る。
fn is_safe_group_col(group_col: &str) -> bool {
    // 列参照は既知の 2 種を厳密一致で許可（`entity_col` の matches! と同型）。CASE 式は
    // `case_from_preds` が label 側の単一引用符を debug_assert 済みで注入不可なので shape
    // （`CASE` 始まり）で許可する（CASE 式は label リテラルの単一引用符を正当に含むため、
    // 単一引用符の有無では判定しない）。
    matches!(group_col, "races.venue" | "results.jockey") || group_col.starts_with("CASE")
}

/// 動的キー GROUP BY の成績集計（#350 相性 factor）。`results.<entity_col> = $1` に一致する成績を
/// `group_col`（`races.venue` / `results.jockey` / 距離帯 CASE 等の **code 定数式**）別に集計し、
/// group_col 値をラベルにした `Vec<GroupStat>` を返す。固定ラベル版（`entity_stats` の by_surface 等）と
/// 違い、走った場・騎乗騎手など可変集合をそのまま返す。`entity_col`/`group_col` は SQL に直接埋め込む
/// ため code 定数のみ（`entity_stats` と同じ二重防御）。`value`/`cutoff` はプレースホルダでバインド。
/// cutoff で as-of リーク（`races.date < cutoff`）を防ぐ。
pub(crate) async fn dynamic_group_stats(
    pool: &PgPool,
    entity_col: &str,
    value: &str,
    group_col: &str,
    cutoff: Option<&str>,
) -> crate::error::Result<Vec<GroupStat>> {
    debug_assert!(
        matches!(entity_col, "jockey" | "horse_name"),
        "entity_col must be a known literal, got {entity_col:?}"
    );
    debug_assert!(
        is_safe_group_col(group_col),
        "group_col must be a code-constant column/CASE expr, got {group_col:?}"
    );
    let q = format!(
        r#"
        SELECT
            {group_col} AS k,
            {STATS_AGG_EXPRS}
        FROM results
        INNER JOIN races ON races.race_id = results.race_id
        WHERE results.{entity_col} = $1
          AND results.finishing_position IS NOT NULL
          AND ({group_col}) IS NOT NULL
          {date}
        GROUP BY {group_col}
        "#,
        date = date_lt_pred(cutoff, "$2"),
    );
    let mut query =
        sqlx::query_as::<_, (String, i64, i64, i64, i64)>(sqlx::AssertSqlSafe(q)).bind(value);
    if let Some(d) = cutoff {
        query = query.bind(d);
    }
    let rows = query.fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|(label, s, w, p, sh)| group_stat_from_row(&label, (s, w, p, sh)))
        .collect())
}

/// [`dynamic_group_stats`] のバッチ版（#350）。`results.<entity_col> = ANY($1)` を `(entity, group_col)` で
/// 一括集計し、`entity 値 -> Vec<GroupStat>` を返す（per-item と同一の述語・集計式・ラベル）。
/// 結果に現れない entity は map に載らない（呼び出し側が空 `Vec` を補完する）。
pub(crate) async fn dynamic_group_stats_batch(
    pool: &PgPool,
    entity_col: &str,
    values: &[&str],
    group_col: &str,
    cutoff: Option<&str>,
) -> crate::error::Result<HashMap<String, Vec<GroupStat>>> {
    debug_assert!(
        matches!(entity_col, "jockey" | "horse_name"),
        "entity_col must be a known literal, got {entity_col:?}"
    );
    debug_assert!(
        is_safe_group_col(group_col),
        "group_col must be a code-constant column/CASE expr, got {group_col:?}"
    );
    // 空 values ガード（`entity_stats_batch` と対称）。`= ANY('{}')` は 0 行で無害だが、
    // 無駄なクエリ発行を避けて呼び出し規約を揃える。
    if values.is_empty() {
        return Ok(HashMap::new());
    }
    let q = format!(
        r#"
        SELECT
            results.{entity_col} AS e,
            {group_col} AS k,
            {STATS_AGG_EXPRS}
        FROM results
        INNER JOIN races ON races.race_id = results.race_id
        WHERE results.{entity_col} = ANY($1)
          AND results.finishing_position IS NOT NULL
          AND ({group_col}) IS NOT NULL
          {date}
        GROUP BY results.{entity_col}, {group_col}
        "#,
        date = date_lt_pred(cutoff, "$2"),
    );
    let mut query =
        sqlx::query_as::<_, (String, String, i64, i64, i64, i64)>(sqlx::AssertSqlSafe(q))
            .bind(values);
    if let Some(d) = cutoff {
        query = query.bind(d);
    }
    let rows = query.fetch_all(pool).await?;
    let mut out: HashMap<String, Vec<GroupStat>> = HashMap::new();
    for (entity, label, s, w, p, sh) in rows {
        out.entry(entity)
            .or_default()
            .push(group_stat_from_row(&label, (s, w, p, sh)));
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

#[cfg(test)]
mod tests {
    use super::{
        STATS_AGG_EXPRS, STATS_AGG_SELECT, case_from_bands, case_from_preds, or_from_preds,
    };

    #[test]
    fn case_from_preds_builds_when_then_chain() {
        let sql = case_from_preds(&[("A", "x = 1"), ("B", "x = 2")]);
        assert_eq!(sql, "CASE WHEN x = 1 THEN 'A' WHEN x = 2 THEN 'B' END");
    }

    #[test]
    fn case_from_preds_empty_does_not_panic() {
        // アーム 0 個でも Rust ビルダーは panic せず `CASE END` を返す（この生成 SQL 自体は
        // WHEN/ELSE を欠き不正。実運用では常に非空の code 定数を渡す前提）。
        assert_eq!(case_from_preds(&[]), "CASE END");
    }

    #[test]
    fn stats_agg_exprs_stay_in_sync_with_select() {
        // STATS_AGG_SELECT は const 連結ができず集計式リテラルを別に保持する。両者がドリフトすると
        // ベース率が静かに食い違うため、SELECT 本体が EXPRS を（空白差を無視して）包含することをロックする。
        let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            norm(STATS_AGG_SELECT).contains(&norm(STATS_AGG_EXPRS)),
            "STATS_AGG_SELECT must embed STATS_AGG_EXPRS verbatim (drift detected)"
        );
    }

    #[test]
    fn or_from_preds_wraps_and_joins_with_or() {
        let sql = or_from_preds(&[("A", "x = 1"), ("B", "x BETWEEN 2 AND 3")]);
        assert_eq!(sql, "(x = 1) OR (x BETWEEN 2 AND 3)");
    }

    #[test]
    fn or_from_preds_single_has_no_or() {
        assert_eq!(or_from_preds(&[("A", "x = 1")]), "(x = 1)");
    }

    #[test]
    fn is_safe_group_col_accepts_known_and_rejects_injection() {
        use super::{DISTANCE_BAND_PREDS, case_from_preds, is_safe_group_col};
        // 現行の全呼び出し形（列参照 2 種 + case_from_preds 生成の CASE 式）を許可。
        assert!(is_safe_group_col("races.venue"));
        assert!(is_safe_group_col("results.jockey"));
        assert!(is_safe_group_col(&case_from_preds(DISTANCE_BAND_PREDS)));
        // 単一引用符（リテラル注入）・未知の列は拒否。
        assert!(!is_safe_group_col("races.venue; DROP TABLE races"));
        assert!(!is_safe_group_col("'injected'"));
        assert!(!is_safe_group_col("results.horse_name"));
    }

    #[test]
    fn case_from_bands_builds_between_chain() {
        let sql = case_from_bands("f", &[("多", 14, 99), ("少", 1, 9)]);
        assert_eq!(
            sql,
            "CASE WHEN f BETWEEN 14 AND 99 THEN '多' WHEN f BETWEEN 1 AND 9 THEN '少' END"
        );
    }
}
