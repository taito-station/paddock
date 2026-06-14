use chrono::NaiveDate;
use paddock_domain::{Surface, Venue};
use paddock_use_case::repository::CourseStatsRow;
use sqlx::SqlitePool;

use crate::error::Result;

use super::sql::{GATE_GROUPS, STATS_AGG_SELECT, date_lt_pred, group_stat_from_row};

/// コース（場×距離×馬場）の枠順別成績を返す。集計本体・枠順グループは stats 共通ヘルパを使う（#85）。
/// `as_of = Some(d)` のとき `races.date < d` を付与する（バックテストのリーク防止）。
/// course_stats は既に `races` を JOIN しているため述語追加のみでよい。
pub async fn course_stats(
    pool: &SqlitePool,
    venue: Venue,
    distance: u32,
    surface: Surface,
    as_of: Option<NaiveDate>,
) -> Result<CourseStatsRow> {
    let cutoff = as_of.map(|d| d.format("%Y-%m-%d").to_string());
    // course は venue/distance/surface の多列バインド（distance は整数）で `entity_stats`/`fetch_agg`
    // の単一文字列値パターンに乗らないため、共通定数（STATS_AGG_SELECT/GATE_GROUPS）のみ使い
    // バインドは手書きする。
    let date = date_lt_pred(cutoff.as_deref(), "?4");
    let mut by_gate_group = Vec::with_capacity(GATE_GROUPS.len());
    for (label, predicate) in GATE_GROUPS {
        let q = format!(
            "{STATS_AGG_SELECT} WHERE races.venue = ?1 AND races.distance = ?2 \
             AND races.surface = ?3 AND results.finishing_position IS NOT NULL \
             AND {predicate} {date}"
        );
        let mut query = sqlx::query_as(sqlx::AssertSqlSafe(&*q))
            .bind(venue.as_jp())
            .bind(distance as i64)
            .bind(surface.as_str());
        if let Some(d) = &cutoff {
            query = query.bind(d);
        }
        let row: (i64, i64, i64, i64) = query.fetch_one(pool).await?;
        by_gate_group.push(group_stat_from_row(label, row));
    }
    Ok(CourseStatsRow {
        venue: venue.as_jp().to_string(),
        distance,
        surface: surface.as_str().to_string(),
        by_gate_group,
    })
}
