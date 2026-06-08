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
/// `races.date < before` でバックテスト時のリークを防ぐ。pdf/netkeiba 双方の成績を対象とする
/// （実際の前走を取りたいため source は絞らない）。同一馬の同一実レースが pdf と netkeiba で同日
/// 二重登録されうるため、`race_id` 降順を第 2 ソートキーにして `LIMIT 1` の選択を決定的にする
/// （非決定だと前走フォームが run ごとにブレる）。
pub async fn find_recent_runs(
    pool: &SqlitePool,
    name: &HorseName,
    before: NaiveDate,
    limit: u32,
) -> Result<Vec<(NaiveDate, HorseResult)>> {
    let before_str = before.format("%Y-%m-%d").to_string();

    let rows: Vec<RecentRow> = sqlx::query_as(
        r#"
        SELECT
            races.date,
            results.race_id, results.finishing_position, results.status,
            results.gate_num, results.horse_num, results.horse_name,
            results.horse_id, results.jockey, results.trainer,
            results.time_seconds, results.margin, results.odds,
            results.horse_weight, results.weight_change, results.weight_carried,
            results.popularity
        FROM results
        INNER JOIN races ON races.race_id = results.race_id
        WHERE results.horse_name = $1
          AND races.date < $2
        ORDER BY races.date DESC, results.race_id DESC
        LIMIT $3
        "#,
    )
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
