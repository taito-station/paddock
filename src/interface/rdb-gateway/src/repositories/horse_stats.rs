use chrono::NaiveDate;
use paddock_domain::{DatedCounts, HorseName};
use paddock_use_case::repository::{GroupStat, HorseRecencyStats, HorseStatsRow, RecencySeries};
use sqlx::PgPool;

use crate::error::{Error, Result};

use super::sql::date_lt_pred;

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

    let overall = overall_stat(pool, n, cutoff.as_deref()).await?;

    Ok(HorseStatsRow {
        horse_name: n.to_string(),
        by_surface,
        by_distance_band,
        by_gate_group,
        by_track_condition,
        by_popularity_band,
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
