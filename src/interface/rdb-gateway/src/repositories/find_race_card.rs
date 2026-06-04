use paddock_domain::{GateNum, HorseEntry, HorseName, HorseNum, JockeyName, RaceCard, RaceId, Surface, Venue};
use sqlx::SqlitePool;

use crate::error::Result;

pub async fn find_race_card(pool: &SqlitePool, race_id: &RaceId) -> Result<Option<RaceCard>> {
    let card_row: Option<(String, String, i64, i64, i64, String, i64)> = sqlx::query_as(
        r#"
        SELECT race_id, venue, round, day, race_num, surface, distance
        FROM race_cards
        WHERE race_id = $1
        "#,
    )
    .bind(race_id.value())
    .fetch_optional(pool)
    .await?;

    let Some((race_id_str, venue_str, round, day, race_num, surface_str, distance)) = card_row
    else {
        return Ok(None);
    };

    let entry_rows: Vec<(i64, i64, String, Option<String>)> = sqlx::query_as(
        r#"
        SELECT gate_num, horse_num, horse_name, jockey
        FROM horse_entries
        WHERE race_id = $1
        ORDER BY horse_num
        "#,
    )
    .bind(&race_id_str)
    .fetch_all(pool)
    .await?;

    let race_id = RaceId::try_from(race_id_str.as_str())?;
    let venue = Venue::try_from(venue_str.as_str())?;
    let surface = Surface::try_from(surface_str.as_str())?;

    let mut entries = Vec::with_capacity(entry_rows.len());
    for (gate_num, horse_num, horse_name, jockey) in entry_rows {
        entries.push(HorseEntry {
            gate_num: GateNum::try_from(gate_num as u32)?,
            horse_num: HorseNum::try_from(horse_num as u32)?,
            horse_name: HorseName::try_from(horse_name.as_str())?,
            jockey: jockey
                .map(|j| JockeyName::try_from(j.as_str()))
                .transpose()?,
        });
    }

    Ok(Some(RaceCard {
        race_id,
        venue,
        round: round as u32,
        day: day as u32,
        race_num: race_num as u32,
        surface,
        distance: distance as u32,
        entries,
    }))
}
