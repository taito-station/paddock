mod header;
mod row;

use chrono::NaiveDate;
use paddock_domain::{
    FinishingPosition, GateNum, HorseName, HorseNum, HorseResult, JockeyName, Race, RaceId,
    TimeSeconds, TrainerName,
};

use crate::error::{Error, Result};

pub use header::RaceHeader;
pub use row::RawRow;

pub fn parse_text(text: &str) -> Result<Vec<Race>> {
    let blocks = split_into_race_blocks(text);
    let mut races = Vec::with_capacity(blocks.len());
    for block in blocks {
        if let Some(race) = build_race_from_block(&block)? {
            races.push(race);
        }
    }
    Ok(races)
}

/// A race block is the slice of lines from one race-start marker to the next.
fn split_into_race_blocks(text: &str) -> Vec<Vec<String>> {
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let starts: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            if header::is_race_start_line(l) {
                Some(i)
            } else {
                None
            }
        })
        .collect();
    if starts.is_empty() {
        return Vec::new();
    }
    let mut blocks = Vec::with_capacity(starts.len());
    for (idx, &start) in starts.iter().enumerate() {
        let end = starts.get(idx + 1).copied().unwrap_or(lines.len());
        blocks.push(lines[start..end].to_vec());
    }
    blocks
}

fn build_race_from_block(lines: &[String]) -> Result<Option<Race>> {
    let header = match header::parse_header(lines)? {
        Some(h) => h,
        None => return Ok(None),
    };

    let race_id_str = format!(
        "{}-{}-{}-{}-{}R",
        header.year,
        header.round,
        header.venue.as_slug(),
        header.day,
        header.race_num
    );
    let race_id = RaceId::try_from(race_id_str)?;

    let chunks = row::collect_chunks(lines);
    let field_size = header::find_field_size(lines);

    let mut results = Vec::with_capacity(chunks.len());
    let mut finisher_count: u32 = 0;
    let mut valid_chunk_idx: u32 = 0;
    for chunk in chunks.iter() {
        let raw = row::parse_chunk(chunk);
        let gate_num = match raw.gate.and_then(|n| GateNum::try_from(n).ok()) {
            Some(g) => g,
            None => continue,
        };
        let horse_num = match raw.horse_num.and_then(|n| HorseNum::try_from(n).ok()) {
            Some(h) => h,
            None => continue,
        };
        let horse_name = match raw
            .horse_name
            .as_deref()
            .and_then(|s| HorseName::try_from(s).ok())
        {
            Some(n) => n,
            None => continue,
        };

        let scratched = chunk
            .iter()
            .any(|l| l.contains("競走除外") || l.contains("出走取消"));
        let beyond_finishers = field_size.is_some_and(|n| valid_chunk_idx >= n);
        valid_chunk_idx += 1;

        let finishing_position = if scratched || beyond_finishers {
            None
        } else {
            finisher_count += 1;
            Some(FinishingPosition::try_from(finisher_count)?)
        };

        let jockey = raw
            .jockey
            .as_deref()
            .and_then(|s| JockeyName::try_from(s).ok());
        let trainer = raw
            .trainer
            .as_deref()
            .and_then(|s| TrainerName::try_from(s).ok());
        let time_seconds = raw
            .time_str
            .as_deref()
            .and_then(|s| TimeSeconds::try_from_mss_str(s).ok());

        results.push(HorseResult {
            finishing_position,
            gate_num,
            horse_num,
            horse_name,
            jockey,
            trainer,
            time_seconds,
            margin: raw.margin,
            odds: raw.odds,
            horse_weight: raw.horse_weight,
            weight_change: raw.weight_change,
        });
    }

    let race = Race {
        race_id,
        date: header.date,
        venue: header.venue,
        round: header.round,
        day: header.day,
        race_num: header.race_num,
        surface: header.surface,
        distance: header.distance,
        track_condition: header.track_condition,
        weather: header.weather,
        results,
    };
    Ok(Some(race))
}

#[allow(dead_code)]
fn naive_date(year: i32, month: u32, day: u32) -> Result<NaiveDate> {
    NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| Error::Parse(format!("invalid date {year}-{month}-{day}")))
}
