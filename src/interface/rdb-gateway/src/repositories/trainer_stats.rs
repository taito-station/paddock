use chrono::NaiveDate;
use paddock_domain::TrainerName;
use paddock_use_case::repository::{GroupStat, TrainerStatsRow};
use sqlx::SqlitePool;

use crate::error::Result;

use super::sql::date_lt_pred;

/// `as_of = Some(d)` のとき各サブクエリに `races.date < d` を付与する（バックテストのリーク防止）。
/// overall / 枠順別は `FROM results` 単独のため `INNER JOIN races` を足す（`results.race_id` は
/// `NOT NULL REFERENCES races` なので行数は不変、`as_of = None` の結果は従来と一致）。
pub async fn trainer_stats(
    pool: &SqlitePool,
    name: &TrainerName,
    as_of: Option<NaiveDate>,
) -> Result<TrainerStatsRow> {
    let n = name.value();
    let cutoff = as_of.map(|d| d.format("%Y-%m-%d").to_string());

    let overall_q = format!(
        r#"
        SELECT
            COUNT(*) AS starts,
            SUM(CASE WHEN results.finishing_position = 1 THEN 1 ELSE 0 END) AS wins,
            SUM(CASE WHEN results.finishing_position IN (1,2) THEN 1 ELSE 0 END) AS places,
            SUM(CASE WHEN results.finishing_position IN (1,2,3) THEN 1 ELSE 0 END) AS shows
        FROM results
        INNER JOIN races ON races.race_id = results.race_id
        WHERE results.trainer = $1
          AND results.finishing_position IS NOT NULL
          {date}
        "#,
        date = date_lt_pred(cutoff.as_deref(), "?2"),
    );
    let mut overall_query = sqlx::query_as(&overall_q).bind(n);
    if let Some(d) = &cutoff {
        overall_query = overall_query.bind(d);
    }
    let overall: (i64, i64, i64, i64) = overall_query.fetch_one(pool).await?;

    let by_surface = group_by_surface(pool, n, cutoff.as_deref()).await?;
    let by_gate_group = group_by_gate(pool, n, cutoff.as_deref()).await?;

    Ok(TrainerStatsRow {
        trainer_name: n.to_string(),
        overall: GroupStat {
            label: "全体".to_string(),
            starts: overall.0 as u32,
            wins: overall.1 as u32,
            places: overall.2 as u32,
            shows: overall.3 as u32,
        },
        by_surface,
        by_gate_group,
    })
}

async fn group_by_surface(
    pool: &SqlitePool,
    trainer: &str,
    cutoff: Option<&str>,
) -> Result<Vec<GroupStat>> {
    let keys: &[(&str, &str)] = &[("turf", "芝"), ("dirt", "ダート")];
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
            WHERE results.trainer = $1
              AND results.finishing_position IS NOT NULL
              AND races.surface = ?2
              {date}
            "#,
            date = date_lt_pred(cutoff, "?3"),
        );
        let mut query = sqlx::query_as(&q).bind(trainer).bind(*key);
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
    trainer: &str,
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
            WHERE results.trainer = $1
              AND results.finishing_position IS NOT NULL
              AND {predicate}
              {date}
            "#,
            date = date_lt_pred(cutoff, "?2"),
        );
        let mut query = sqlx::query_as(&q).bind(trainer);
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
