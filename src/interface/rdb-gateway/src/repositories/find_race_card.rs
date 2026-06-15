use chrono::NaiveDate;
use paddock_domain::{
    GateNum, HorseEntry, HorseName, HorseNum, JockeyName, RaceCard, RaceId, Surface, TrainerName,
    Venue,
};
use sqlx::SqlitePool;

use crate::error::{Error, Result};

/// horse_entries の 1 行（gate_num, horse_num, horse_name, jockey, trainer, weight_carried）。
type EntryRow = (
    i64,
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<f64>,
);

#[derive(sqlx::FromRow)]
struct CardRow {
    race_id: String,
    date: Option<String>,
    venue: String,
    round: i64,
    day: i64,
    race_num: i64,
    surface: String,
    distance: i64,
}

pub async fn find_race_card(pool: &SqlitePool, race_id: &RaceId) -> Result<Option<RaceCard>> {
    let card_row: Option<CardRow> = sqlx::query_as(
        r#"
        SELECT race_id, date, venue, round, day, race_num, surface, distance
        FROM race_cards
        WHERE race_id = $1
        "#,
    )
    .bind(race_id.value())
    .fetch_optional(pool)
    .await?;

    let Some(row) = card_row else {
        return Ok(None);
    };
    let CardRow {
        race_id: race_id_str,
        date: date_str,
        venue: venue_str,
        round,
        day,
        race_num,
        surface: surface_str,
        distance,
    } = row;

    let entry_rows: Vec<EntryRow> = sqlx::query_as(
        r#"
        SELECT gate_num, horse_num, horse_name, jockey, trainer, weight_carried
        FROM horse_entries
        WHERE race_id = $1
        ORDER BY horse_num
        "#,
    )
    .bind(race_id.value())
    .fetch_all(pool)
    .await?;

    // DB 値を再パースして domain 型バリデーションを通す（入力の race_id をそのまま使わない理由:
    // DB 値と入力が一致することを明示的に保証するため）
    let race_id = RaceId::try_from(race_id_str.as_str())?;
    let venue = Venue::try_from(venue_str.as_str())?;
    let surface = Surface::try_from(surface_str.as_str())?;
    // date 列は migration 20260606000003 で追加。新規取り込みは必ず設定するが、
    // 旧データで成績にも紐づかず backfill されなかった行は NULL になり得るため明示エラーにする。
    let date_str = date_str.ok_or_else(|| {
        Error::Data(format!(
            "race_card {} の date が未設定です",
            race_id.value()
        ))
    })?;
    let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
        .map_err(|e| Error::Data(format!("race_card date '{date_str}' のパースに失敗: {e}")))?;

    let mut entries = Vec::with_capacity(entry_rows.len());
    for (gate_num, horse_num, horse_name, jockey, trainer, weight_carried) in entry_rows {
        entries.push(HorseEntry {
            gate_num: GateNum::try_from(gate_num as u32)?,
            horse_num: HorseNum::try_from(horse_num as u32)?,
            horse_name: HorseName::try_from(horse_name.as_str())?,
            jockey: jockey
                .map(|j| JockeyName::try_from(j.as_str()))
                .transpose()?,
            trainer: trainer
                .map(|t| TrainerName::try_from(t.as_str()))
                .transpose()?,
            weight_carried,
        });
    }

    // round / day / race_num / distance は save_race_card で書き込み済みの値を
    // as キャストで戻す。書き込み側でバリデーション済みなのでサイレント wrap は起きない。
    Ok(Some(RaceCard {
        race_id,
        date,
        venue,
        round: round as u32,
        day: day as u32,
        race_num: race_num as u32,
        surface,
        distance: distance as u32,
        entries,
    }))
}
