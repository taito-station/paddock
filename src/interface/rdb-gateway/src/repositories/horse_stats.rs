use chrono::NaiveDate;
use paddock_domain::HorseName;
use paddock_use_case::repository::{GroupStat, HorseStatsRow};
use sqlx::SqlitePool;

use crate::error::Result;

/// `as_of = Some(d)` のとき各サブクエリに `races.date < d` を付与し、その日付より前の成績のみを
/// 集計する（バックテストのリーク防止）。`results.race_id` は `NOT NULL REFERENCES races` のため、
/// `INNER JOIN races` を無条件に足しても行数は変わらず、`as_of = None`（全期間）の結果は従来と一致する。
pub async fn horse_stats(
    pool: &SqlitePool,
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

/// `races.date < ?N` の述語片を返す（`as_of` が無ければ空文字）。日付値はプレースホルダで
/// バインドするため、ここでは文字列連結に値を含めない。
fn date_pred(cutoff: Option<&str>, placeholder: &str) -> String {
    if cutoff.is_some() {
        format!("AND races.date < {placeholder}")
    } else {
        String::new()
    }
}

async fn overall_stat(
    pool: &SqlitePool,
    horse_name: &str,
    cutoff: Option<&str>,
) -> Result<GroupStat> {
    let q = format!(
        r#"
        SELECT
            COUNT(*) AS starts,
            SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END) AS wins,
            SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END) AS places,
            SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END) AS shows
        FROM results
        INNER JOIN races ON races.race_id = results.race_id
        WHERE results.horse_name = $1
          AND results.finishing_position IS NOT NULL
          {date}
        "#,
        date = date_pred(cutoff, "?2"),
    );
    let mut query = sqlx::query_as(&q).bind(horse_name);
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
    pool: &SqlitePool,
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
                SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END) AS wins,
                SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END) AS places,
                SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END) AS shows
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.horse_name = $1
              AND results.finishing_position IS NOT NULL
              AND {column} = ?2
              {date}
            "#,
            date = date_pred(cutoff, "?3"),
        );
        let mut query = sqlx::query_as(&q).bind(horse_name).bind(*key);
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
    pool: &SqlitePool,
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
                SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END) AS wins,
                SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END) AS places,
                SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END) AS shows
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.horse_name = $1
              AND results.finishing_position IS NOT NULL
              AND {predicate}
              {date}
            "#,
            date = date_pred(cutoff, "?2"),
        );
        let mut query = sqlx::query_as(&q).bind(horse_name);
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
    pool: &SqlitePool,
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
                SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END) AS wins,
                SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END) AS places,
                SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END) AS shows
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.horse_name = $1
              AND results.finishing_position IS NOT NULL
              AND results.popularity IS NOT NULL
              AND results.{predicate}
              {date}
            "#,
            date = date_pred(cutoff, "?2"),
        );
        let mut query = sqlx::query_as(&q).bind(horse_name);
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

async fn group_by_gate(
    pool: &SqlitePool,
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
                SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END) AS wins,
                SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END) AS places,
                SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END) AS shows
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.horse_name = $1
              AND results.finishing_position IS NOT NULL
              AND {predicate}
              {date}
            "#,
            date = date_pred(cutoff, "?2"),
        );
        let mut query = sqlx::query_as(&q).bind(horse_name);
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
