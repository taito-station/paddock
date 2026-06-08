use paddock_use_case::RaceOddsRecord;
use sqlx::SqlitePool;

use crate::error::Result;

/// 1 レース分のオッズを 1 トランザクションで upsert する。
/// 主キー `(race_id, bet_type, combination_key)` で衝突した行は最新値で上書きする。
pub async fn save_race_odds(pool: &SqlitePool, record: &RaceOddsRecord) -> Result<()> {
    let mut tx = pool.begin().await?;

    let fetched_at = record.fetched_at.to_rfc3339();
    for row in &record.rows {
        sqlx::query(
            r#"
            INSERT INTO race_odds
                (race_id, bet_type, combination_key, odds, odds_high, popularity, fetched_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT(race_id, bet_type, combination_key) DO UPDATE SET
                odds       = excluded.odds,
                odds_high  = excluded.odds_high,
                popularity = excluded.popularity,
                fetched_at = excluded.fetched_at
            "#,
        )
        .bind(record.race_id.value())
        .bind(&row.bet_type)
        .bind(&row.combination_key)
        .bind(row.odds)
        .bind(row.odds_high)
        .bind(row.popularity.map(|p| p as i64))
        .bind(&fetched_at)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}
