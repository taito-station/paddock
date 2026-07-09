use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::{DatedCounts, HorseName};
use paddock_use_case::repository::{GroupStat, HorseRecencyStats, HorseStatsRow, RecencySeries};
use sqlx::PgPool;

use crate::error::{Error, Result};

use super::sql::{date_lt_pred, dynamic_group_stats, dynamic_group_stats_batch};

/// `as_of = Some(d)` のとき各サブクエリに `races.date < d` を付与し、その日付より前の成績のみを
/// 集計する（バックテストのリーク防止）。`results.race_id` は `NOT NULL REFERENCES races` のため、
/// `INNER JOIN races` を無条件に足しても行数は変わらず、`as_of = None`（全期間）の結果は従来と一致する。
pub async fn horse_stats(
    pool: &PgPool,
    name: &HorseName,
    as_of: Option<NaiveDate>,
) -> Result<HorseStatsRow> {
    let n = name.value();
    let cutoff = as_of.map(|d| d.format("%Y-%m-%d").to_string());

    let by_surface = group_by(
        pool,
        n,
        "races.surface",
        &[("turf", "芝"), ("dirt", "ダート")],
        cutoff.as_deref(),
    )
    .await?;

    let by_distance_band = group_by_distance_band(pool, n, cutoff.as_deref()).await?;

    let by_gate_group = group_by_gate(pool, n, cutoff.as_deref()).await?;

    let by_track_condition = group_by(
        pool,
        n,
        "races.track_condition",
        &[
            ("良", "良"),
            ("稍重", "稍重"),
            ("重", "重"),
            ("不良", "不良"),
        ],
        cutoff.as_deref(),
    )
    .await?;

    let by_popularity_band = group_by_popularity_band(pool, n, cutoff.as_deref()).await?;

    // #350 相性 factor: 馬×競馬場（venue）・騎手×馬コンビ（この馬の戦績を騎手別に分割）。動的キー GROUP BY。
    let by_venue =
        dynamic_group_stats(pool, "horse_name", n, "races.venue", cutoff.as_deref()).await?;
    let by_jockey =
        dynamic_group_stats(pool, "horse_name", n, "results.jockey", cutoff.as_deref()).await?;

    let overall = overall_stat(pool, n, cutoff.as_deref()).await?;

    Ok(HorseStatsRow {
        horse_name: n.to_string(),
        by_surface,
        by_distance_band,
        by_gate_group,
        by_track_condition,
        by_popularity_band,
        by_venue,
        by_jockey,
        overall,
    })
}

async fn overall_stat(pool: &PgPool, horse_name: &str, cutoff: Option<&str>) -> Result<GroupStat> {
    let q = format!(
        r#"
        SELECT
            COUNT(*) AS starts,
            COALESCE(SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END), 0) AS wins,
            COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END), 0) AS places,
            COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END), 0) AS shows
        FROM results
        INNER JOIN races ON races.race_id = results.race_id
        WHERE results.horse_name = $1
          AND results.finishing_position IS NOT NULL
          {date}
        "#,
        date = date_lt_pred(cutoff, "$2"),
    );
    let mut query = sqlx::query_as(sqlx::AssertSqlSafe(q)).bind(horse_name);
    if let Some(d) = cutoff {
        query = query.bind(d);
    }
    let row: (i64, i64, i64, i64) = query.fetch_one(pool).await?;
    Ok(GroupStat {
        label: "全体".to_string(),
        starts: row.0 as u32,
        wins: row.1 as u32,
        places: row.2 as u32,
        shows: row.3 as u32,
    })
}

async fn group_by(
    pool: &PgPool,
    horse_name: &str,
    column: &str,
    keys: &[(&str, &str)],
    cutoff: Option<&str>,
) -> Result<Vec<GroupStat>> {
    let mut stats = Vec::with_capacity(keys.len());
    for (key, label) in keys {
        let q = format!(
            r#"
            SELECT
                COUNT(*) AS starts,
                COALESCE(SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END), 0) AS wins,
                COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END), 0) AS places,
                COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END), 0) AS shows
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.horse_name = $1
              AND results.finishing_position IS NOT NULL
              AND {column} = $2
              {date}
            "#,
            date = date_lt_pred(cutoff, "$3"),
        );
        let mut query = sqlx::query_as(sqlx::AssertSqlSafe(q))
            .bind(horse_name)
            .bind(*key);
        if let Some(d) = cutoff {
            query = query.bind(d);
        }
        let row: (i64, i64, i64, i64) = query.fetch_one(pool).await?;
        stats.push(GroupStat {
            label: label.to_string(),
            starts: row.0 as u32,
            wins: row.1 as u32,
            places: row.2 as u32,
            shows: row.3 as u32,
        });
    }
    Ok(stats)
}

async fn group_by_distance_band(
    pool: &PgPool,
    horse_name: &str,
    cutoff: Option<&str>,
) -> Result<Vec<GroupStat>> {
    let bands: &[(&str, &str)] = &[
        ("〜1400m", "races.distance <= 1400"),
        ("1500〜1800m", "races.distance BETWEEN 1500 AND 1800"),
        ("1900〜2200m", "races.distance BETWEEN 1900 AND 2200"),
        ("2300m〜", "races.distance >= 2300"),
    ];
    let mut stats = Vec::with_capacity(bands.len());
    for (label, predicate) in bands {
        let q = format!(
            r#"
            SELECT
                COUNT(*) AS starts,
                COALESCE(SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END), 0) AS wins,
                COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END), 0) AS places,
                COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END), 0) AS shows
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.horse_name = $1
              AND results.finishing_position IS NOT NULL
              AND {predicate}
              {date}
            "#,
            date = date_lt_pred(cutoff, "$2"),
        );
        let mut query = sqlx::query_as(sqlx::AssertSqlSafe(q)).bind(horse_name);
        if let Some(d) = cutoff {
            query = query.bind(d);
        }
        let row: (i64, i64, i64, i64) = query.fetch_one(pool).await?;
        stats.push(GroupStat {
            label: label.to_string(),
            starts: row.0 as u32,
            wins: row.1 as u32,
            places: row.2 as u32,
            shows: row.3 as u32,
        });
    }
    Ok(stats)
}

async fn group_by_popularity_band(
    pool: &PgPool,
    horse_name: &str,
    cutoff: Option<&str>,
) -> Result<Vec<GroupStat>> {
    let bands: &[(&str, &str)] = &[
        ("1人気", "popularity = 1"),
        ("2-5人気", "popularity BETWEEN 2 AND 5"),
        ("6-10人気", "popularity BETWEEN 6 AND 10"),
        ("11人気-", "popularity >= 11"),
    ];
    let mut stats = Vec::with_capacity(bands.len());
    for (label, predicate) in bands {
        let q = format!(
            r#"
            SELECT
                COUNT(*) AS starts,
                COALESCE(SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END), 0) AS wins,
                COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END), 0) AS places,
                COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END), 0) AS shows
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.horse_name = $1
              AND results.finishing_position IS NOT NULL
              AND results.popularity IS NOT NULL
              AND results.{predicate}
              {date}
            "#,
            date = date_lt_pred(cutoff, "$2"),
        );
        let mut query = sqlx::query_as(sqlx::AssertSqlSafe(q)).bind(horse_name);
        if let Some(d) = cutoff {
            query = query.bind(d);
        }
        let row: (i64, i64, i64, i64) = query.fetch_one(pool).await?;
        stats.push(GroupStat {
            label: label.to_string(),
            starts: row.0 as u32,
            wins: row.1 as u32,
            places: row.2 as u32,
            shows: row.3 as u32,
        });
    }
    Ok(stats)
}

/// recency 重み付け（#75 Phase B）用に、馬の成績を「カテゴリ × ラベル別の日付付き系列」で返す。
/// build_factors が使う horse 系 3 factor（芝ダ・距離帯・馬場状態）に対応する。各系列は
/// `races.date` で GROUP BY したカウントで、domain 側が時間減衰を掛ける。`as_of` は集計と同じ
/// リーク防止（`races.date < as_of`）。
pub async fn horse_recency(
    pool: &PgPool,
    name: &HorseName,
    as_of: Option<NaiveDate>,
) -> Result<HorseRecencyStats> {
    let n = name.value();
    let cutoff = as_of.map(|d| d.format("%Y-%m-%d").to_string());

    let by_surface = dated_group_by(
        pool,
        n,
        "races.surface",
        &[("turf", "芝"), ("dirt", "ダート")],
        cutoff.as_deref(),
    )
    .await?;
    let by_distance_band = dated_distance_band(pool, n, cutoff.as_deref()).await?;
    let by_track_condition = dated_group_by(
        pool,
        n,
        "races.track_condition",
        &[
            ("良", "良"),
            ("稍重", "稍重"),
            ("重", "重"),
            ("不良", "不良"),
        ],
        cutoff.as_deref(),
    )
    .await?;

    Ok(HorseRecencyStats {
        by_surface,
        by_distance_band,
        by_track_condition,
    })
}

/// 集計の生 row `(date 文字列, starts, wins, places, shows)` を [`DatedCounts`] へ変換する。
/// `races.date` は `YYYY-MM-DD` テキストで保存されている前提（集計の `date_lt_pred` と同じ）。
fn rows_to_dated(rows: Vec<(String, i64, i64, i64, i64)>) -> Result<Vec<DatedCounts>> {
    rows.into_iter()
        .map(|(d, starts, wins, places, shows)| {
            let date = NaiveDate::parse_from_str(&d, "%Y-%m-%d")
                .map_err(|e| Error::Data(format!("invalid races.date '{d}': {e}")))?;
            Ok(DatedCounts {
                date,
                starts: starts as u32,
                wins: wins as u32,
                places: places as u32,
                shows: shows as u32,
            })
        })
        .collect()
}

/// `column = key` でフィルタした成績を `races.date` 別に集計し、ラベルごとの日付系列で返す。
async fn dated_group_by(
    pool: &PgPool,
    horse_name: &str,
    column: &str,
    keys: &[(&str, &str)],
    cutoff: Option<&str>,
) -> Result<Vec<RecencySeries>> {
    // `column` は SQL に直接埋め込むため既知リテラルのみ許す（`entity_stats` と同じ二重防御）。
    // `keys`・`horse_name`・`cutoff` はプレースホルダでバインドする（インジェクション安全）。
    debug_assert!(
        matches!(column, "races.surface" | "races.track_condition"),
        "column must be a known literal, got {column:?}"
    );
    let mut out = Vec::with_capacity(keys.len());
    for (key, label) in keys {
        let q = format!(
            r#"
            SELECT
                races.date AS d,
                COUNT(*) AS starts,
                COALESCE(SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END), 0) AS wins,
                COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END), 0) AS places,
                COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END), 0) AS shows
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.horse_name = $1
              AND results.finishing_position IS NOT NULL
              AND {column} = $2
              {date}
            GROUP BY races.date
            ORDER BY races.date
            "#,
            date = date_lt_pred(cutoff, "$3"),
        );
        let mut query = sqlx::query_as::<_, (String, i64, i64, i64, i64)>(sqlx::AssertSqlSafe(q))
            .bind(horse_name)
            .bind(*key);
        if let Some(d) = cutoff {
            query = query.bind(d);
        }
        let rows = query.fetch_all(pool).await?;
        out.push(RecencySeries {
            label: label.to_string(),
            runs: rows_to_dated(rows)?,
        });
    }
    Ok(out)
}

/// 距離帯（band 述語）でフィルタした成績を `races.date` 別に集計し、帯ごとの日付系列で返す。
async fn dated_distance_band(
    pool: &PgPool,
    horse_name: &str,
    cutoff: Option<&str>,
) -> Result<Vec<RecencySeries>> {
    let bands: &[(&str, &str)] = &[
        ("〜1400m", "races.distance <= 1400"),
        ("1500〜1800m", "races.distance BETWEEN 1500 AND 1800"),
        ("1900〜2200m", "races.distance BETWEEN 1900 AND 2200"),
        ("2300m〜", "races.distance >= 2300"),
    ];
    let mut out = Vec::with_capacity(bands.len());
    for (label, predicate) in bands {
        let q = format!(
            r#"
            SELECT
                races.date AS d,
                COUNT(*) AS starts,
                COALESCE(SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END), 0) AS wins,
                COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END), 0) AS places,
                COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END), 0) AS shows
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.horse_name = $1
              AND results.finishing_position IS NOT NULL
              AND {predicate}
              {date}
            GROUP BY races.date
            ORDER BY races.date
            "#,
            date = date_lt_pred(cutoff, "$2"),
        );
        let mut query = sqlx::query_as::<_, (String, i64, i64, i64, i64)>(sqlx::AssertSqlSafe(q))
            .bind(horse_name);
        if let Some(d) = cutoff {
            query = query.bind(d);
        }
        let rows = query.fetch_all(pool).await?;
        out.push(RecencySeries {
            label: label.to_string(),
            runs: rows_to_dated(rows)?,
        });
    }
    Ok(out)
}

async fn group_by_gate(
    pool: &PgPool,
    horse_name: &str,
    cutoff: Option<&str>,
) -> Result<Vec<GroupStat>> {
    let groups: &[(&str, &str)] = &[
        ("Inner (1-3)", "results.gate_num BETWEEN 1 AND 3"),
        ("Middle (4-6)", "results.gate_num BETWEEN 4 AND 6"),
        ("Outer (7-8)", "results.gate_num BETWEEN 7 AND 8"),
    ];
    let mut stats = Vec::with_capacity(groups.len());
    for (label, predicate) in groups {
        let q = format!(
            r#"
            SELECT
                COUNT(*) AS starts,
                COALESCE(SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END), 0) AS wins,
                COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END), 0) AS places,
                COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END), 0) AS shows
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.horse_name = $1
              AND results.finishing_position IS NOT NULL
              AND {predicate}
              {date}
            "#,
            date = date_lt_pred(cutoff, "$2"),
        );
        let mut query = sqlx::query_as(sqlx::AssertSqlSafe(q)).bind(horse_name);
        if let Some(d) = cutoff {
            query = query.bind(d);
        }
        let row: (i64, i64, i64, i64) = query.fetch_one(pool).await?;
        stats.push(GroupStat {
            label: label.to_string(),
            starts: row.0 as u32,
            wins: row.1 as u32,
            places: row.2 as u32,
            shows: row.3 as u32,
        });
    }
    Ok(stats)
}

// ===== バッチ版（#196 backtest の馬ごと N+1 解消） =====
//
// 各 per-label クエリを「`results.horse_name = ANY($1)` + `SELECT/GROUP BY results.horse_name`」
// に変えて全馬一括で引く。WHERE 述語・JOIN・集計式は per-item と完全同値で、Rust 側で
// horse_name にグルーピングし、クエリに現れない馬は全ゼロの `GroupStat` を合成する。これにより
// 返却 `HorseStatsRow` は per-item の `horse_stats` を全馬に対して呼んだ結果と一致する。

/// per-item の集計 SELECT に `results.horse_name` を加え、`WHERE results.horse_name = ANY($1) AND <tail>`
/// と `GROUP BY results.horse_name` で全馬一括集計し、`horse_name -> (starts, wins, places, shows)` を返す。
/// `tail` は per-item の WHERE 後半（`finishing_position IS NOT NULL ...` 以降、`date_lt_pred` まで）を
/// そのまま渡す。`extra_binds` は `$2..` に対応する追加バインド（key 等）。cutoff は末尾に積む。
async fn grouped_agg(
    pool: &PgPool,
    names: &[&str],
    tail: &str,
    extra_binds: &[&str],
    cutoff: Option<&str>,
) -> Result<HashMap<String, (i64, i64, i64, i64)>> {
    // 空 `names` ガードは持たない（`ANY($1)` は空配列でも 0 件を返すだけで安全だが）。呼び出し側
    // `horse_stats_batch` / `horse_recency_batch` が `unique.is_empty()` で空を弾くため、ここに空
    // `names` は渡らない前提（entity_stats_batch 側は別途空ガードあり）。
    let q = format!(
        r#"
        SELECT
            results.horse_name AS horse_name,
            COUNT(*) AS starts,
            COALESCE(SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END), 0) AS wins,
            COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END), 0) AS places,
            COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END), 0) AS shows
        FROM results
        INNER JOIN races ON races.race_id = results.race_id
        WHERE results.horse_name = ANY($1)
          {tail}
        GROUP BY results.horse_name
        "#,
    );
    let mut query =
        sqlx::query_as::<_, (String, i64, i64, i64, i64)>(sqlx::AssertSqlSafe(q)).bind(names);
    for b in extra_binds {
        query = query.bind(*b);
    }
    if let Some(d) = cutoff {
        query = query.bind(d);
    }
    let rows = query.fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|(name, s, w, p, sh)| (name, (s, w, p, sh)))
        .collect())
}

/// グルーピング結果 `map` から `name` の `label` 行を取り出す。無ければ全ゼロを合成する
/// （per-item で行が無い＝COUNT(*)=0 のケースと同一の `GroupStat`）。
fn group_stat_for(
    map: &HashMap<String, (i64, i64, i64, i64)>,
    name: &str,
    label: &str,
) -> GroupStat {
    let (starts, wins, places, shows) = map.get(name).copied().unwrap_or((0, 0, 0, 0));
    GroupStat {
        label: label.to_string(),
        starts: starts as u32,
        wins: wins as u32,
        places: places as u32,
        shows: shows as u32,
    }
}

/// 複数馬の [`HorseStatsRow`] を per-item `horse_stats` と同値でまとめて返す（#196）。
pub async fn horse_stats_batch(
    pool: &PgPool,
    names: &[HorseName],
    as_of: Option<NaiveDate>,
) -> Result<HashMap<HorseName, HorseStatsRow>> {
    // 重複名は 1 回だけ引く（ANY 母集合・返却 map とも一意でよい）。
    let mut unique: Vec<HorseName> = Vec::new();
    for n in names {
        if !unique.contains(n) {
            unique.push(n.clone());
        }
    }
    if unique.is_empty() {
        return Ok(HashMap::new());
    }
    let name_strs: Vec<&str> = unique.iter().map(|n| n.value()).collect();
    let cutoff = as_of.map(|d| d.format("%Y-%m-%d").to_string());

    // ラベル定義は per-item と同一順・同一文字列。
    let surface_keys: &[(&str, &str)] = &[("turf", "芝"), ("dirt", "ダート")];
    let track_keys: &[(&str, &str)] = &[
        ("良", "良"),
        ("稍重", "稍重"),
        ("重", "重"),
        ("不良", "不良"),
    ];
    let distance_bands: &[(&str, &str)] = &[
        ("〜1400m", "races.distance <= 1400"),
        ("1500〜1800m", "races.distance BETWEEN 1500 AND 1800"),
        ("1900〜2200m", "races.distance BETWEEN 1900 AND 2200"),
        ("2300m〜", "races.distance >= 2300"),
    ];
    let popularity_bands: &[(&str, &str)] = &[
        ("1人気", "popularity = 1"),
        ("2-5人気", "popularity BETWEEN 2 AND 5"),
        ("6-10人気", "popularity BETWEEN 6 AND 10"),
        ("11人気-", "popularity >= 11"),
    ];
    let gate_groups: &[(&str, &str)] = &[
        ("Inner (1-3)", "results.gate_num BETWEEN 1 AND 3"),
        ("Middle (4-6)", "results.gate_num BETWEEN 4 AND 6"),
        ("Outer (7-8)", "results.gate_num BETWEEN 7 AND 8"),
    ];

    // overall: per-item の `finishing_position IS NOT NULL {date}`（cutoff は $2）。
    let overall_map = grouped_agg(
        pool,
        &name_strs,
        &format!(
            "AND results.finishing_position IS NOT NULL {date}",
            date = date_lt_pred(cutoff.as_deref(), "$2"),
        ),
        &[],
        cutoff.as_deref(),
    )
    .await?;

    // by_surface / by_track_condition: `AND <column> = $2 {date}`（cutoff は $3）。
    let mut surface_maps = Vec::with_capacity(surface_keys.len());
    for (key, label) in surface_keys {
        let map = grouped_agg(
            pool,
            &name_strs,
            &format!(
                "AND results.finishing_position IS NOT NULL AND races.surface = $2 {date}",
                date = date_lt_pred(cutoff.as_deref(), "$3"),
            ),
            &[key],
            cutoff.as_deref(),
        )
        .await?;
        surface_maps.push((*label, map));
    }
    let mut track_maps = Vec::with_capacity(track_keys.len());
    for (key, label) in track_keys {
        let map = grouped_agg(
            pool,
            &name_strs,
            &format!(
                "AND results.finishing_position IS NOT NULL AND races.track_condition = $2 {date}",
                date = date_lt_pred(cutoff.as_deref(), "$3"),
            ),
            &[key],
            cutoff.as_deref(),
        )
        .await?;
        track_maps.push((*label, map));
    }

    // by_distance_band / by_gate_group: predicate 直埋め（per-item と同じ、cutoff は $2）。
    let mut distance_maps = Vec::with_capacity(distance_bands.len());
    for (label, predicate) in distance_bands {
        let map = grouped_agg(
            pool,
            &name_strs,
            &format!(
                "AND results.finishing_position IS NOT NULL AND {predicate} {date}",
                date = date_lt_pred(cutoff.as_deref(), "$2"),
            ),
            &[],
            cutoff.as_deref(),
        )
        .await?;
        distance_maps.push((*label, map));
    }
    let mut gate_maps = Vec::with_capacity(gate_groups.len());
    for (label, predicate) in gate_groups {
        let map = grouped_agg(
            pool,
            &name_strs,
            &format!(
                "AND results.finishing_position IS NOT NULL AND {predicate} {date}",
                date = date_lt_pred(cutoff.as_deref(), "$2"),
            ),
            &[],
            cutoff.as_deref(),
        )
        .await?;
        gate_maps.push((*label, map));
    }
    // by_popularity_band: per-item は `popularity IS NOT NULL AND results.{predicate}`。
    let mut popularity_maps = Vec::with_capacity(popularity_bands.len());
    for (label, predicate) in popularity_bands {
        let map = grouped_agg(
            pool,
            &name_strs,
            &format!(
                "AND results.finishing_position IS NOT NULL \
                 AND results.popularity IS NOT NULL AND results.{predicate} {date}",
                date = date_lt_pred(cutoff.as_deref(), "$2"),
            ),
            &[],
            cutoff.as_deref(),
        )
        .await?;
        popularity_maps.push((*label, map));
    }

    // #350 相性 factor（バッチ版）。venue（races.venue）・combo（results.jockey）とも動的キーで
    // 全馬一括集計。結果に現れない馬は空 Vec（該当実績なし＝factor None）で補完する。
    let venue_map = dynamic_group_stats_batch(
        pool,
        "horse_name",
        &name_strs,
        "races.venue",
        cutoff.as_deref(),
    )
    .await?;
    let jockey_map = dynamic_group_stats_batch(
        pool,
        "horse_name",
        &name_strs,
        "results.jockey",
        cutoff.as_deref(),
    )
    .await?;

    let mut out = HashMap::with_capacity(unique.len());
    for name in &unique {
        let n = name.value();
        out.insert(
            name.clone(),
            HorseStatsRow {
                horse_name: n.to_string(),
                by_surface: surface_maps
                    .iter()
                    .map(|(label, m)| group_stat_for(m, n, label))
                    .collect(),
                by_distance_band: distance_maps
                    .iter()
                    .map(|(label, m)| group_stat_for(m, n, label))
                    .collect(),
                by_gate_group: gate_maps
                    .iter()
                    .map(|(label, m)| group_stat_for(m, n, label))
                    .collect(),
                by_track_condition: track_maps
                    .iter()
                    .map(|(label, m)| group_stat_for(m, n, label))
                    .collect(),
                by_popularity_band: popularity_maps
                    .iter()
                    .map(|(label, m)| group_stat_for(m, n, label))
                    .collect(),
                by_venue: venue_map.get(n).cloned().unwrap_or_default(),
                by_jockey: jockey_map.get(n).cloned().unwrap_or_default(),
                overall: group_stat_for(&overall_map, n, "全体"),
            },
        );
    }
    Ok(out)
}

/// 複数馬の [`HorseRecencyStats`] を per-item `horse_recency` と同値でまとめて返す（#196）。
/// 各 label の系列は per-item と同じく「その label にマッチした (horse, date) のみ」で、ゼロ行は
/// 混ぜない。`GROUP BY races.date` を `GROUP BY results.horse_name, races.date` に変えて全馬一括化する。
pub async fn horse_recency_batch(
    pool: &PgPool,
    names: &[HorseName],
    as_of: Option<NaiveDate>,
) -> Result<HashMap<HorseName, HorseRecencyStats>> {
    let mut unique: Vec<HorseName> = Vec::new();
    for n in names {
        if !unique.contains(n) {
            unique.push(n.clone());
        }
    }
    if unique.is_empty() {
        return Ok(HashMap::new());
    }
    let name_strs: Vec<&str> = unique.iter().map(|n| n.value()).collect();
    let cutoff = as_of.map(|d| d.format("%Y-%m-%d").to_string());

    let surface_keys: &[(&str, &str)] = &[("turf", "芝"), ("dirt", "ダート")];
    let track_keys: &[(&str, &str)] = &[
        ("良", "良"),
        ("稍重", "稍重"),
        ("重", "重"),
        ("不良", "不良"),
    ];
    let distance_bands: &[(&str, &str)] = &[
        ("〜1400m", "races.distance <= 1400"),
        ("1500〜1800m", "races.distance BETWEEN 1500 AND 1800"),
        ("1900〜2200m", "races.distance BETWEEN 1900 AND 2200"),
        ("2300m〜", "races.distance >= 2300"),
    ];

    // by_surface / by_track_condition: `AND <column> = $2 {date}`（cutoff は $3）。
    let mut surface_maps = Vec::with_capacity(surface_keys.len());
    for (key, label) in surface_keys {
        let map = dated_grouped(
            pool,
            &name_strs,
            &format!(
                "AND results.finishing_position IS NOT NULL AND races.surface = $2 {date}",
                date = date_lt_pred(cutoff.as_deref(), "$3"),
            ),
            &[key],
            cutoff.as_deref(),
        )
        .await?;
        surface_maps.push((*label, map));
    }
    let mut track_maps = Vec::with_capacity(track_keys.len());
    for (key, label) in track_keys {
        let map = dated_grouped(
            pool,
            &name_strs,
            &format!(
                "AND results.finishing_position IS NOT NULL AND races.track_condition = $2 {date}",
                date = date_lt_pred(cutoff.as_deref(), "$3"),
            ),
            &[key],
            cutoff.as_deref(),
        )
        .await?;
        track_maps.push((*label, map));
    }
    // by_distance_band: predicate 直埋め（cutoff は $2）。
    let mut distance_maps = Vec::with_capacity(distance_bands.len());
    for (label, predicate) in distance_bands {
        let map = dated_grouped(
            pool,
            &name_strs,
            &format!(
                "AND results.finishing_position IS NOT NULL AND {predicate} {date}",
                date = date_lt_pred(cutoff.as_deref(), "$2"),
            ),
            &[],
            cutoff.as_deref(),
        )
        .await?;
        distance_maps.push((*label, map));
    }

    let mut out = HashMap::with_capacity(unique.len());
    for name in &unique {
        let n = name.value();
        let series = |maps: &[(&str, HashMap<String, Vec<DatedCounts>>)]| -> Vec<RecencySeries> {
            maps.iter()
                .map(|(label, m)| RecencySeries {
                    label: label.to_string(),
                    runs: m.get(n).cloned().unwrap_or_default(),
                })
                .collect()
        };
        out.insert(
            name.clone(),
            HorseRecencyStats {
                by_surface: series(&surface_maps),
                by_distance_band: series(&distance_maps),
                by_track_condition: series(&track_maps),
            },
        );
    }
    Ok(out)
}

/// recency 用の dated 集計を全馬一括で引く。per-item の `GROUP BY races.date` を
/// `GROUP BY results.horse_name, races.date` に変え、`horse_name -> Vec<DatedCounts>`（date 昇順）を返す。
/// per-item と同じく、その label にマッチした (horse, date) の行のみを含む（ゼロ行は混ぜない）。
async fn dated_grouped(
    pool: &PgPool,
    names: &[&str],
    tail: &str,
    extra_binds: &[&str],
    cutoff: Option<&str>,
) -> Result<HashMap<String, Vec<DatedCounts>>> {
    let q = format!(
        r#"
        SELECT
            results.horse_name AS horse_name,
            races.date AS d,
            COUNT(*) AS starts,
            COALESCE(SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END), 0) AS wins,
            COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END), 0) AS places,
            COALESCE(SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END), 0) AS shows
        FROM results
        INNER JOIN races ON races.race_id = results.race_id
        WHERE results.horse_name = ANY($1)
          {tail}
        GROUP BY results.horse_name, races.date
        ORDER BY results.horse_name, races.date
        "#,
    );
    let mut query =
        sqlx::query_as::<_, (String, String, i64, i64, i64, i64)>(sqlx::AssertSqlSafe(q))
            .bind(names);
    for b in extra_binds {
        query = query.bind(*b);
    }
    if let Some(d) = cutoff {
        query = query.bind(d);
    }
    let rows = query.fetch_all(pool).await?;
    let mut out: HashMap<String, Vec<DatedCounts>> = HashMap::new();
    for (name, d, starts, wins, places, shows) in rows {
        let date = NaiveDate::parse_from_str(&d, "%Y-%m-%d")
            .map_err(|e| Error::Data(format!("invalid races.date '{d}': {e}")))?;
        out.entry(name).or_default().push(DatedCounts {
            date,
            starts: starts as u32,
            wins: wins as u32,
            places: places as u32,
            shows: shows as u32,
        });
    }
    Ok(out)
}
