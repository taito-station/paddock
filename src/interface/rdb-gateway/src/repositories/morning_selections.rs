use chrono::NaiveDate;
use paddock_use_case::repository::MorningSelection;
use sqlx::PgPool;

use crate::error::{Error, Result};

#[derive(sqlx::FromRow)]
struct MorningSelectionRow {
    race_id: String,
    date: String,
    axis: i64,
    partners: Vec<i64>,
    konsen_band: Vec<i64>,
    odds_fetched_at: String,
    computed_at: String,
}

fn horse_num(race_id: &str, n: i64) -> Result<u32> {
    u32::try_from(n)
        .map_err(|_| Error::Data(format!("morning_selections {race_id}: 馬番 {n} が不正")))
}

impl TryFrom<MorningSelectionRow> for MorningSelection {
    type Error = Error;

    fn try_from(row: MorningSelectionRow) -> Result<Self> {
        let date = NaiveDate::parse_from_str(&row.date, "%Y-%m-%d").map_err(|e| {
            Error::Data(format!(
                "morning_selections {}: date {:?}: {e}",
                row.race_id, row.date
            ))
        })?;
        let nums = |v: Vec<i64>| -> Result<Vec<u32>> {
            v.into_iter().map(|n| horse_num(&row.race_id, n)).collect()
        };
        Ok(MorningSelection {
            axis: horse_num(&row.race_id, row.axis)?,
            partners: nums(row.partners)?,
            konsen_band: nums(row.konsen_band)?,
            race_id: row.race_id,
            date,
            odds_fetched_at: row.odds_fetched_at,
            computed_at: row.computed_at,
        })
    }
}

/// 指定開催日の朝の選定を `race_id` 昇順で返す（#767）。
pub async fn find_morning_selections_by_date(
    pool: &PgPool,
    date: NaiveDate,
) -> Result<Vec<MorningSelection>> {
    let rows: Vec<MorningSelectionRow> = sqlx::query_as(
        "SELECT race_id, date, axis, partners, konsen_band, odds_fetched_at, computed_at \
         FROM morning_selections WHERE date = $1 ORDER BY race_id ASC",
    )
    .bind(date.format("%Y-%m-%d").to_string())
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(MorningSelection::try_from).collect()
}

/// 1 レースの朝の選定を返す（#767）。
pub async fn find_morning_selection(
    pool: &PgPool,
    race_id: &str,
) -> Result<Option<MorningSelection>> {
    let row: Option<MorningSelectionRow> = sqlx::query_as(
        "SELECT race_id, date, axis, partners, konsen_band, odds_fetched_at, computed_at \
         FROM morning_selections WHERE race_id = $1",
    )
    .bind(race_id)
    .fetch_optional(pool)
    .await?;
    row.map(MorningSelection::try_from).transpose()
}

/// 朝の選定を先勝ちで保存する（#767）。同じレースの行が既にあれば何もしない（上書きしない）。
pub async fn save_morning_selection(pool: &PgPool, s: &MorningSelection) -> Result<()> {
    let to_i64 = |v: &[u32]| v.iter().map(|&n| i64::from(n)).collect::<Vec<i64>>();
    sqlx::query(
        "INSERT INTO morning_selections \
         (race_id, date, axis, partners, konsen_band, odds_fetched_at, computed_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (race_id) DO NOTHING",
    )
    .bind(&s.race_id)
    .bind(s.date.format("%Y-%m-%d").to_string())
    .bind(i64::from(s.axis))
    .bind(to_i64(&s.partners))
    .bind(to_i64(&s.konsen_band))
    .bind(&s.odds_fetched_at)
    .bind(&s.computed_at)
    .execute(pool)
    .await?;
    Ok(())
}
