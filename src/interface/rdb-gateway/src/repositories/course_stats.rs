use paddock_domain::{Surface, Venue};
use paddock_use_case::repository::{CourseStatsRow, GroupStat};
use sqlx::SqlitePool;

use crate::error::Result;

pub async fn course_stats(
    pool: &SqlitePool,
    venue: Venue,
    distance: u32,
    surface: Surface,
) -> Result<CourseStatsRow> {
    let groups: &[(&str, &str)] = &[
        ("Inner (1-3)", "results.gate_num BETWEEN 1 AND 3"),
        ("Middle (4-6)", "results.gate_num BETWEEN 4 AND 6"),
        ("Outer (7-8)", "results.gate_num BETWEEN 7 AND 8"),
    ];
    let mut by_gate_group = Vec::with_capacity(groups.len());
    for (label, predicate) in groups {
        let q = format!(
            r#"
            SELECT
                COUNT(*) AS starts,
                SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END) AS wins,
                SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END) AS places
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE races.venue = $1
              AND races.distance = $2
              AND races.surface = $3
              AND results.finishing_position IS NOT NULL
              AND {predicate}
            "#
        );
        let row: (i64, i64, i64) = sqlx::query_as(&q)
            .bind(venue.as_jp())
            .bind(distance as i64)
            .bind(surface.as_str())
            .fetch_one(pool)
            .await?;
        by_gate_group.push(GroupStat {
            label: label.to_string(),
            starts: row.0 as u32,
            wins: row.1 as u32,
            places: row.2 as u32,
        });
    }
    Ok(CourseStatsRow {
        venue: venue.as_jp().to_string(),
        distance,
        surface: surface.as_str().to_string(),
        by_gate_group,
    })
}
