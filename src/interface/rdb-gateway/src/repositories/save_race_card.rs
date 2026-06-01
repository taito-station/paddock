use paddock_domain::RaceCard;
use sqlx::SqlitePool;

use crate::error::Result;

pub async fn save_race_card(pool: &SqlitePool, card: &RaceCard) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO race_cards (race_id, venue, round, day, race_num, surface, distance)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT(race_id) DO UPDATE SET
            venue    = excluded.venue,
            round    = excluded.round,
            day      = excluded.day,
            race_num = excluded.race_num,
            surface  = excluded.surface,
            distance = excluded.distance
        "#,
    )
    .bind(card.race_id.value())
    .bind(card.venue.as_jp())
    .bind(card.round as i64)
    .bind(card.day as i64)
    .bind(card.race_num as i64)
    .bind(card.surface.as_str())
    .bind(card.distance as i64)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM horse_entries WHERE race_id = $1")
        .bind(card.race_id.value())
        .execute(&mut *tx)
        .await?;

    for entry in &card.entries {
        sqlx::query(
            r#"
            INSERT INTO horse_entries (race_id, gate_num, horse_num, horse_name, jockey)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT(race_id, horse_num) DO UPDATE SET
                gate_num   = excluded.gate_num,
                horse_name = excluded.horse_name,
                jockey     = excluded.jockey
            "#,
        )
        .bind(card.race_id.value())
        .bind(entry.gate_num.value() as i64)
        .bind(entry.horse_num.value() as i64)
        .bind(entry.horse_name.value())
        .bind(entry.jockey.as_ref().map(|j| j.value().to_string()))
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}
