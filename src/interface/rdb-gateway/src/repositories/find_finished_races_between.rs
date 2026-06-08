use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::{
    FinishingPosition, GateNum, HorseId, HorseName, HorseNum, HorseResult, JockeyName, Race,
    RaceId, ResultStatus, Surface, TimeSeconds, TrackCondition, TrainerName, Venue, Weather,
};
use sqlx::SqlitePool;

use crate::error::Result;

#[derive(sqlx::FromRow)]
struct RaceRow {
    race_id: String,
    date: String,
    venue: String,
    round: i64,
    day: i64,
    race_num: i64,
    surface: String,
    distance: i64,
    track_condition: Option<String>,
    weather: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ResultRow {
    race_id: String,
    finishing_position: Option<i64>,
    status: String,
    gate_num: i64,
    horse_num: i64,
    horse_name: String,
    horse_id: Option<String>,
    jockey: Option<String>,
    trainer: Option<String>,
    time_seconds: Option<f64>,
    margin: Option<String>,
    odds: Option<f64>,
    horse_weight: Option<i64>,
    weight_change: Option<i64>,
    weight_carried: Option<f64>,
    popularity: Option<i64>,
}

/// 指定期間 `[from, to]`（両端含む）の確定済みレースを `results` 付きで取得する。
///
/// `races.source='pdf'` かつ着順ありの `results` を 1 件以上含むレースのみを対象とし、
/// `date`・`race_num` 昇順に返す。`results` は着順 `None`（除外・失格等）の行も含めて全件返す
/// （馬単位の欠落は use-case 層で扱う）。`from > to` のときは `BETWEEN` が空集合となり空 Vec を返す。
///
/// N+1 を避けるため、レース一覧と results をそれぞれ 1 クエリで取得し、Rust 側で race_id 突合する。
pub async fn find_finished_races_between(
    pool: &SqlitePool,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<Race>> {
    let from_str = from.format("%Y-%m-%d").to_string();
    let to_str = to.format("%Y-%m-%d").to_string();

    let race_rows: Vec<RaceRow> = sqlx::query_as(
        r#"
        SELECT race_id, date, venue, round, day, race_num,
               surface, distance, track_condition, weather
        FROM races
        WHERE date BETWEEN $1 AND $2
          AND source = 'pdf'
          AND EXISTS (
              SELECT 1
              FROM results
              WHERE results.race_id = races.race_id
                AND results.finishing_position IS NOT NULL
          )
        ORDER BY date ASC, race_num ASC, race_id ASC
        "#,
    )
    .bind(&from_str)
    .bind(&to_str)
    .fetch_all(pool)
    .await?;

    if race_rows.is_empty() {
        return Ok(Vec::new());
    }

    let result_rows: Vec<ResultRow> = sqlx::query_as(
        r#"
        SELECT
            results.race_id, results.finishing_position, results.status,
            results.gate_num, results.horse_num, results.horse_name,
            results.horse_id, results.jockey, results.trainer,
            results.time_seconds, results.margin, results.odds,
            results.horse_weight, results.weight_change, results.weight_carried,
            results.popularity
        FROM results
        INNER JOIN races ON races.race_id = results.race_id
        WHERE races.date BETWEEN $1 AND $2
          AND races.source = 'pdf'
        ORDER BY results.race_id ASC, results.horse_num ASC
        "#,
    )
    .bind(&from_str)
    .bind(&to_str)
    .fetch_all(pool)
    .await?;

    // race_id ごとに results を束ねる。
    let mut results_by_race: HashMap<String, Vec<HorseResult>> = HashMap::new();
    for row in result_rows {
        let race_id = row.race_id.clone();
        let result = row_to_result(row)?;
        results_by_race.entry(race_id).or_default().push(result);
    }

    let mut races = Vec::with_capacity(race_rows.len());
    for row in race_rows {
        let track_condition = row
            .track_condition
            .map(|s| TrackCondition::try_from(s.as_str()))
            .transpose()?;
        let weather = row
            .weather
            .map(|s| Weather::try_from(s.as_str()))
            .transpose()?;
        let results = results_by_race.remove(&row.race_id).unwrap_or_default();

        races.push(Race {
            race_id: RaceId::try_from(row.race_id.as_str())?,
            date: NaiveDate::parse_from_str(&row.date, "%Y-%m-%d")
                .map_err(|e| crate::Error::Data(format!("invalid race date: {e}")))?,
            venue: Venue::try_from(row.venue.as_str())?,
            round: row.round as u32,
            day: row.day as u32,
            race_num: row.race_num as u32,
            surface: Surface::try_from(row.surface.as_str())?,
            distance: row.distance as u32,
            track_condition,
            weather,
            results,
        });
    }

    Ok(races)
}

fn row_to_result(row: ResultRow) -> Result<HorseResult> {
    let finishing_position = row
        .finishing_position
        .map(|p| FinishingPosition::try_from(p as u32))
        .transpose()?;
    let horse_id = row
        .horse_id
        .map(|s| HorseId::try_from(s.as_str()))
        .transpose()?;
    let jockey = row
        .jockey
        .map(|s| JockeyName::try_from(s.as_str()))
        .transpose()?;
    let trainer = row
        .trainer
        .map(|s| TrainerName::try_from(s.as_str()))
        .transpose()?;
    let time_seconds = row.time_seconds.map(TimeSeconds::try_from).transpose()?;

    Ok(HorseResult {
        finishing_position,
        status: ResultStatus::try_from(row.status.as_str())?,
        gate_num: GateNum::try_from(row.gate_num as u32)?,
        horse_num: HorseNum::try_from(row.horse_num as u32)?,
        horse_name: HorseName::try_from(row.horse_name.as_str())?,
        horse_id,
        jockey,
        trainer,
        time_seconds,
        margin: row.margin,
        odds: row.odds,
        horse_weight: row.horse_weight.map(|w| w as u32),
        weight_change: row.weight_change.map(|w| w as i32),
        weight_carried: row.weight_carried,
        popularity: row.popularity.map(|p| p as u32),
    })
}
