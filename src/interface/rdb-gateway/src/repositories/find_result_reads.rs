use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::RaceId;
use paddock_use_case::FinishEntry;
use sqlx::PgPool;

use crate::error::Result;

/// 指定日の結果確定レース（`results` に着順 `finishing_position IS NOT NULL` 行が 1 件以上）を
/// `race_id → true` で返す（#381）。確定レースのみ含める（未確定は呼び出し側で false 既定）。
/// `races.date` で日を絞る。同日取り込みしたレースは `races` 行を持つため拾える。
pub async fn find_result_confirmed_by_date(
    pool: &PgPool,
    date: NaiveDate,
) -> Result<HashMap<RaceId, bool>> {
    let date_str = date.format("%Y-%m-%d").to_string();
    let rows: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT races.race_id
        FROM races
        WHERE races.date = $1
          AND EXISTS (
              SELECT 1
              FROM results
              WHERE results.race_id = races.race_id
                AND results.finishing_position IS NOT NULL
          )
        "#,
    )
    .bind(&date_str)
    .fetch_all(pool)
    .await?;

    let mut map = HashMap::with_capacity(rows.len());
    for (race_id,) in rows {
        map.insert(RaceId::try_from(race_id.as_str())?, true);
    }
    Ok(map)
}

/// 指定日の各レースの上位着順（`finishing_position <= 3`・着順昇順）を `race_id → Vec<FinishEntry>` で返す。
/// 3 着同着で 4 件以上返りうる（件数可変）。確定レースのみキーに含まれる。
pub async fn find_top_finishes_by_date(
    pool: &PgPool,
    date: NaiveDate,
) -> Result<HashMap<RaceId, Vec<FinishEntry>>> {
    let date_str = date.format("%Y-%m-%d").to_string();
    let rows: Vec<(String, i64, i64, String)> = sqlx::query_as(
        r#"
        SELECT results.race_id, results.finishing_position, results.horse_num, results.horse_name
        FROM results
        INNER JOIN races
            ON races.race_id = results.race_id
        WHERE races.date = $1
          AND results.finishing_position IS NOT NULL
          AND results.finishing_position <= 3
        ORDER BY results.race_id ASC, results.finishing_position ASC, results.horse_num ASC
        "#,
    )
    .bind(&date_str)
    .fetch_all(pool)
    .await?;

    let mut map: HashMap<RaceId, Vec<FinishEntry>> = HashMap::new();
    for (race_id, position, horse_num, horse_name) in rows {
        map.entry(RaceId::try_from(race_id.as_str())?)
            .or_default()
            .push(FinishEntry {
                position: position as u32,
                horse_num: horse_num as u32,
                horse_name,
            });
    }
    Ok(map)
}

/// 指定レースの `馬番 → 着順`（board 用・#381）。着順が入っている馬のみ含む。
pub async fn find_finishing_positions(
    pool: &PgPool,
    race_id: &RaceId,
) -> Result<HashMap<u32, u32>> {
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        r#"
        SELECT horse_num, finishing_position
        FROM results
        WHERE race_id = $1
          AND finishing_position IS NOT NULL
        "#,
    )
    .bind(race_id.value())
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(horse_num, position)| (horse_num as u32, position as u32))
        .collect())
}
