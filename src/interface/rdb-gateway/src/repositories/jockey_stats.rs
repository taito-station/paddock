use paddock_domain::JockeyName;
use paddock_use_case::repository::{GroupStat, JockeyStatsRow};
use sqlx::SqlitePool;

use crate::error::Result;

pub async fn jockey_stats(pool: &SqlitePool, name: &JockeyName) -> Result<JockeyStatsRow> {
    let n = name.value();

    let overall: (i64, i64, i64) = sqlx::query_as(
        r#"
        SELECT
            COUNT(*) AS starts,
            SUM(CASE WHEN finishing_position = 1 THEN 1 ELSE 0 END) AS wins,
            SUM(CASE WHEN finishing_position IN (1,2) THEN 1 ELSE 0 END) AS places
        FROM results
        WHERE jockey = $1 AND finishing_position IS NOT NULL
        "#,
    )
    .bind(n)
    .fetch_one(pool)
    .await?;

    let by_surface = group_by_surface(pool, n).await?;
    let by_gate_group = group_by_gate(pool, n).await?;

    Ok(JockeyStatsRow {
        jockey_name: n.to_string(),
        overall: GroupStat {
            label: "全体".to_string(),
            starts: overall.0 as u32,
            wins: overall.1 as u32,
            places: overall.2 as u32,
        },
        by_surface,
        by_gate_group,
    })
}

async fn group_by_surface(pool: &SqlitePool, jockey: &str) -> Result<Vec<GroupStat>> {
    let keys: &[(&str, &str)] = &[("turf", "芝"), ("dirt", "ダート")];
    let mut stats = Vec::with_capacity(keys.len());
    for (key, label) in keys {
        let row: (i64, i64, i64) = sqlx::query_as(
            r#"
            SELECT
                COUNT(*) AS starts,
                SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END) AS wins,
                SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END) AS places
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.jockey = $1
              AND results.finishing_position IS NOT NULL
              AND races.surface = $2
            "#,
        )
        .bind(jockey)
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

async fn group_by_gate(pool: &SqlitePool, jockey: &str) -> Result<Vec<GroupStat>> {
    let groups: &[(&str, &str)] = &[
        ("Inner (1-3)", "gate_num BETWEEN 1 AND 3"),
        ("Middle (4-6)", "gate_num BETWEEN 4 AND 6"),
        ("Outer (7-8)", "gate_num BETWEEN 7 AND 8"),
    ];
    let mut stats = Vec::with_capacity(groups.len());
    for (label, predicate) in groups {
        let q = format!(
            r#"
            SELECT
                COUNT(*) AS starts,
                SUM(CASE WHEN finishing_position = 1 THEN 1 ELSE 0 END) AS wins,
                SUM(CASE WHEN finishing_position IN (1,2) THEN 1 ELSE 0 END) AS places
            FROM results
            WHERE jockey = $1
              AND finishing_position IS NOT NULL
              AND {predicate}
            "#
        );
        let row: (i64, i64, i64) = sqlx::query_as(&q).bind(jockey).fetch_one(pool).await?;
        stats.push(GroupStat {
            label: label.to_string(),
            starts: row.0 as u32,
            wins: row.1 as u32,
            places: row.2 as u32,
        });
    }
    Ok(stats)
}
