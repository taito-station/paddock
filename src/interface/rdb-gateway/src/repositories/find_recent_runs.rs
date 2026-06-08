use chrono::NaiveDate;
use paddock_domain::{HorseName, HorseResult};
use sqlx::SqlitePool;

use super::find_finished_races_between::{ResultRow, row_to_result};
use crate::error::Result;

/// `races.date` と結果カラムをまとめて受けるための行（`ResultRow` を flatten で再利用）。
#[derive(sqlx::FromRow)]
struct RecentRow {
    date: String,
    #[sqlx(flatten)]
    result: ResultRow,
}

/// 指定馬の `before` より前の成績を date 降順で最大 `limit` 件取得する（前走フォーム #31 用）。
///
/// pdf 確定成績(`results`)と netkeiba 近走(`horse_past_runs`)を UNION し、`date < before` で
/// バックテスト時のリークを防ぐ。同一実レースが両ソースに存在する場合は `(date, venue, race_num)`
/// 単位で **pdf を優先**して 1 件に dedup する（pdf=src_rank 0, netkeiba=1）。同点（同ソース・
/// 同一実レースの別 race_id）は `race_id` 降順で決定的に 1 件を選ぶ（前走フォームが run ごとに
/// ブレないようにするため）。
pub async fn find_recent_runs(
    pool: &SqlitePool,
    name: &HorseName,
    before: NaiveDate,
    limit: u32,
) -> Result<Vec<(NaiveDate, HorseResult)>> {
    let before_str = before.format("%Y-%m-%d").to_string();

    let rows: Vec<RecentRow> = sqlx::query_as(
        r#"
        WITH unioned AS (
            SELECT
                races.date AS date, races.venue AS venue, races.race_num AS race_num,
                0 AS src_rank,
                results.race_id AS race_id, results.finishing_position AS finishing_position,
                results.status AS status, results.gate_num AS gate_num,
                results.horse_num AS horse_num, results.horse_name AS horse_name,
                results.horse_id AS horse_id, results.jockey AS jockey,
                results.trainer AS trainer, results.time_seconds AS time_seconds,
                results.margin AS margin, results.odds AS odds,
                results.horse_weight AS horse_weight, results.weight_change AS weight_change,
                results.weight_carried AS weight_carried, results.popularity AS popularity
            FROM results
            INNER JOIN races ON races.race_id = results.race_id
            WHERE results.horse_name = ? AND races.date < ? AND races.source = 'pdf'
            UNION ALL
            SELECT
                date, venue, race_num,
                1 AS src_rank,
                race_id, finishing_position, status, gate_num, horse_num, horse_name,
                horse_id, jockey, NULL AS trainer, time_seconds, margin, odds,
                horse_weight, weight_change, weight_carried, popularity
            FROM horse_past_runs
            WHERE horse_name = ? AND date < ?
        )
        SELECT
            u.date, u.race_id, u.finishing_position, u.status, u.gate_num, u.horse_num,
            u.horse_name, u.horse_id, u.jockey, u.trainer, u.time_seconds, u.margin,
            u.odds, u.horse_weight, u.weight_change, u.weight_carried, u.popularity
        FROM unioned u
        WHERE NOT EXISTS (
            SELECT 1 FROM unioned u2
            WHERE u2.date = u.date AND u2.venue = u.venue AND u2.race_num = u.race_num
              AND (u2.src_rank < u.src_rank
                   OR (u2.src_rank = u.src_rank AND u2.race_id > u.race_id))
        )
        ORDER BY u.date DESC, u.race_id DESC
        LIMIT ?
        "#,
    )
    .bind(name.value())
    .bind(&before_str)
    .bind(name.value())
    .bind(&before_str)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;

    let mut runs = Vec::with_capacity(rows.len());
    for row in rows {
        let date = NaiveDate::parse_from_str(&row.date, "%Y-%m-%d")
            .map_err(|e| crate::Error::Data(format!("invalid race date: {e}")))?;
        runs.push((date, row_to_result(row.result)?));
    }
    Ok(runs)
}
