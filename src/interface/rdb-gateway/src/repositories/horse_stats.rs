use paddock_domain::HorseName;
use paddock_use_case::repository::{GroupStat, HorseStatsRow};
use sqlx::SqlitePool;

use crate::error::Result;

pub async fn horse_stats(pool: &SqlitePool, name: &HorseName) -> Result<HorseStatsRow> {
    let n = name.value();

    let by_surface = group_by(
        pool,
        n,
        "races.surface",
        &[("turf", "芝"), ("dirt", "ダート")],
    )
    .await?;

    let by_distance_band = group_by_distance_band(pool, n).await?;

    let by_gate_group = group_by_gate(pool, n).await?;

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
    )
    .await?;

    let by_popularity_band = group_by_popularity_band(pool, n).await?;

    let overall = overall_stat(pool, n).await?;

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

async fn overall_stat(pool: &SqlitePool, horse_name: &str) -> Result<GroupStat> {
    let row: (i64, i64, i64) = sqlx::query_as(
        r#"
        SELECT
            COUNT(*) AS starts,
            SUM(CASE WHEN finishing_position = 1 THEN 1 ELSE 0 END) AS wins,
            SUM(CASE WHEN finishing_position IN (1,2) THEN 1 ELSE 0 END) AS places
        FROM results
        WHERE horse_name = $1 AND finishing_position IS NOT NULL
        "#,
    )
    .bind(horse_name)
    .fetch_one(pool)
    .await?;
    Ok(GroupStat {
        label: "全体".to_string(),
        starts: row.0 as u32,
        wins: row.1 as u32,
        places: row.2 as u32,
    })
}

async fn group_by(
    pool: &SqlitePool,
    horse_name: &str,
    column: &str,
    keys: &[(&str, &str)],
) -> Result<Vec<GroupStat>> {
    let mut stats = Vec::with_capacity(keys.len());
    for (key, label) in keys {
        let q = format!(
            r#"
            SELECT
                COUNT(*) AS starts,
                SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END) AS wins,
                SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END) AS places
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.horse_name = $1
              AND results.finishing_position IS NOT NULL
              AND {column} = ?2
            "#
        );
        let row: (i64, i64, i64) = sqlx::query_as(&q)
            .bind(horse_name)
            .bind(*key)
            .fetch_one(pool)
            .await?;
        stats.push(GroupStat {
            label: label.to_string(),
            starts: row.0 as u32,
            wins: row.1 as u32,
            places: row.2 as u32,
        });
    }
    Ok(stats)
}

async fn group_by_distance_band(pool: &SqlitePool, horse_name: &str) -> Result<Vec<GroupStat>> {
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
                SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END) AS places
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.horse_name = $1
              AND results.finishing_position IS NOT NULL
              AND {predicate}
            "#
        );
        let row: (i64, i64, i64) = sqlx::query_as(&q).bind(horse_name).fetch_one(pool).await?;
        stats.push(GroupStat {
            label: label.to_string(),
            starts: row.0 as u32,
            wins: row.1 as u32,
            places: row.2 as u32,
        });
    }
    Ok(stats)
}

async fn group_by_popularity_band(pool: &SqlitePool, horse_name: &str) -> Result<Vec<GroupStat>> {
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
                SUM(CASE WHEN finishing_position = 1 THEN 1 ELSE 0 END) AS wins,
                SUM(CASE WHEN finishing_position IN (1,2) THEN 1 ELSE 0 END) AS places
            FROM results
            WHERE horse_name = $1
              AND finishing_position IS NOT NULL
              AND popularity IS NOT NULL
              AND {predicate}
            "#
        );
        let row: (i64, i64, i64) = sqlx::query_as(&q).bind(horse_name).fetch_one(pool).await?;
        stats.push(GroupStat {
            label: label.to_string(),
            starts: row.0 as u32,
            wins: row.1 as u32,
            places: row.2 as u32,
        });
    }
    Ok(stats)
}

async fn group_by_gate(pool: &SqlitePool, horse_name: &str) -> Result<Vec<GroupStat>> {
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
                SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END) AS places
            FROM results
            WHERE results.horse_name = $1
              AND results.finishing_position IS NOT NULL
              AND {predicate}
            "#
        );
        let row: (i64, i64, i64) = sqlx::query_as(&q).bind(horse_name).fetch_one(pool).await?;
        stats.push(GroupStat {
            label: label.to_string(),
            starts: row.0 as u32,
            wins: row.1 as u32,
            places: row.2 as u32,
        });
    }
    Ok(stats)
}
