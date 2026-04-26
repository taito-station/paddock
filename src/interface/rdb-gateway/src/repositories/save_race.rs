use paddock_domain::Race;
use sqlx::SqlitePool;

use crate::error::Result;

pub async fn save_race(pool: &SqlitePool, race: &Race) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO races (race_id, date, venue, round, day, race_num,
                           surface, distance, track_condition, weather)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        ON CONFLICT(race_id) DO UPDATE SET
            date = excluded.date,
            venue = excluded.venue,
            round = excluded.round,
            day = excluded.day,
            race_num = excluded.race_num,
            surface = excluded.surface,
            distance = excluded.distance,
            track_condition = excluded.track_condition,
            weather = excluded.weather
        "#,
    )
    .bind(race.race_id.value())
    .bind(race.date.format("%Y-%m-%d").to_string())
    .bind(race.venue.as_jp())
    .bind(race.round as i64)
    .bind(race.day as i64)
    .bind(race.race_num as i64)
    .bind(race.surface.as_str())
    .bind(race.distance as i64)
    .bind(race.track_condition.as_ref().map(|c| c.as_str()))
    .bind(race.weather.as_ref().map(|w| w.as_str()))
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM results WHERE race_id = $1")
        .bind(race.race_id.value())
        .execute(&mut *tx)
        .await?;

    for r in &race.results {
        sqlx::query(
            r#"
            INSERT INTO results
                (race_id, finishing_position, status, gate_num, horse_num, horse_name,
                 jockey, trainer, time_seconds, margin, odds, horse_weight, weight_change,
                 weight_carried, popularity)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            ON CONFLICT(race_id, horse_num) DO UPDATE SET
                finishing_position = excluded.finishing_position,
                status = excluded.status,
                gate_num = excluded.gate_num,
                horse_name = excluded.horse_name,
                jockey = excluded.jockey,
                trainer = excluded.trainer,
                time_seconds = excluded.time_seconds,
                margin = excluded.margin,
                odds = excluded.odds,
                horse_weight = excluded.horse_weight,
                weight_change = excluded.weight_change,
                weight_carried = excluded.weight_carried,
                popularity = excluded.popularity
            "#,
        )
        .bind(race.race_id.value())
        .bind(r.finishing_position.as_ref().map(|p| p.value() as i64))
        .bind(r.status.as_str())
        .bind(r.gate_num.value() as i64)
        .bind(r.horse_num.value() as i64)
        .bind(r.horse_name.value())
        .bind(r.jockey.as_ref().map(|j| j.value().to_string()))
        .bind(r.trainer.as_ref().map(|t| t.value().to_string()))
        .bind(r.time_seconds.as_ref().map(|t| t.value()))
        .bind(r.margin.clone())
        .bind(r.odds)
        .bind(r.horse_weight.map(|w| w as i64))
        .bind(r.weight_change.map(|w| w as i64))
        .bind(r.weight_carried)
        .bind(r.popularity.map(|p| p as i64))
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}
