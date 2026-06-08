use chrono::NaiveDate;
use paddock_domain::{GateNum, HorseName, HorseNum, JockeyName, Surface};
use paddock_use_case::netkeiba_scraper::{FetchedCard, FetchedEntry};
use regex::Regex;
use scraper::{ElementRef, Html, Selector};

use super::{cell_text, round_day_racenum, venue_from_race_id};
use crate::error::{Error, Result};

/// 出馬表 (`race/shutuba.html`) のHTMLから当日のレースカード（メタ + 全出走馬）を抽出する。
///
/// venue/round/day/race_num は 12 桁 race_id から導出し、surface/distance は
/// `div.RaceData01` のテキスト、開催日は HTML 全体の `YYYY年M月D日` 表記から取る。
/// 各行（`tr.HorseList`）から枠番・馬番・馬名・騎手を読む（馬名は静的 HTML に含まれる）。
pub fn parse_card(html: &str, netkeiba_race_id: &str) -> Result<FetchedCard> {
    let venue = venue_from_race_id(netkeiba_race_id).ok_or_else(|| {
        Error::Parse(format!(
            "JRA 外もしくは不正な race_id: {netkeiba_race_id}"
        ))
    })?;
    let (round, day, race_num) = round_day_racenum(netkeiba_race_id).ok_or_else(|| {
        Error::Parse(format!(
            "race_id から回/日/R を読めません: {netkeiba_race_id}"
        ))
    })?;

    let doc = Html::parse_document(html);

    let (surface, distance) = extract_surface_distance(&doc)?;
    let date = extract_date(html)?;
    let entries = extract_entries(&doc)?;

    Ok(FetchedCard {
        date,
        venue,
        round,
        day,
        race_num,
        surface,
        distance,
        entries,
    })
}

/// `div.RaceData01` のテキストから `芝1600m` 等を読み、Surface と距離(m)に変換する。
/// 障害(障)は対応外として parse error。
fn extract_surface_distance(doc: &Html) -> Result<(Surface, u32)> {
    let data_sel = sel("div.RaceData01")?;
    let text = doc
        .select(&data_sel)
        .next()
        .map(|e| e.text().collect::<String>())
        .ok_or_else(|| Error::Parse("RaceData01 が見つかりません".to_string()))?;

    let re = Regex::new(r"([芝ダ障])\s*(\d{3,4})m")
        .map_err(|e| Error::Parse(format!("invalid surface/distance regex: {e}")))?;
    let caps = re
        .captures(&text)
        .ok_or_else(|| Error::Parse(format!("芝/ダ/距離を読めません: {text:?}")))?;

    let surface = match &caps[1] {
        "芝" => Surface::Turf,
        "ダ" => Surface::Dirt,
        "障" => {
            return Err(Error::Parse(
                "障害レースは対応外です".to_string(),
            ));
        }
        other => return Err(Error::Parse(format!("unknown surface marker: {other}"))),
    };
    let distance: u32 = caps[2]
        .parse()
        .map_err(|_| Error::Parse(format!("invalid distance: {}", &caps[2])))?;
    Ok((surface, distance))
}

/// HTML 全体から最初の `YYYY年M月D日` を開催日として読み取る。
fn extract_date(html: &str) -> Result<NaiveDate> {
    let re = Regex::new(r"(\d{4})年(\d{1,2})月(\d{1,2})日")
        .map_err(|e| Error::Parse(format!("invalid date regex: {e}")))?;
    let caps = re
        .captures(html)
        .ok_or_else(|| Error::Parse("開催日(YYYY年M月D日)が見つかりません".to_string()))?;
    let y: i32 = caps[1].parse().map_err(|_| Error::Parse("invalid year".into()))?;
    let m: u32 = caps[2].parse().map_err(|_| Error::Parse("invalid month".into()))?;
    let d: u32 = caps[3].parse().map_err(|_| Error::Parse("invalid day".into()))?;
    NaiveDate::from_ymd_opt(y, m, d)
        .ok_or_else(|| Error::Parse(format!("不正な開催日: {y}-{m}-{d}")))
}

/// 出馬表テーブルの各行から枠番・馬番・馬名・騎手を抽出する。
fn extract_entries(doc: &Html) -> Result<Vec<FetchedEntry>> {
    let row_sel = sel("table.Shutuba_Table tr.HorseList")?;
    let waku_sel = sel("td[class^=\"Waku\"] span")?;
    let umaban_sel = sel("td[class^=\"Umaban\"]")?;
    let horse_link_sel = sel("td.HorseInfo a[href*=\"/horse/\"]")?;
    let jockey_sel = sel("td.Jockey a")?;

    let mut entries = Vec::new();
    for row in doc.select(&row_sel) {
        let Some(gate_num) = row
            .select(&waku_sel)
            .next()
            .and_then(|c| cell_text(&c.text().collect::<String>()))
            .and_then(|t| t.parse::<u32>().ok())
            .and_then(|n| GateNum::try_from(n).ok())
        else {
            continue;
        };
        let Some(horse_num) = row
            .select(&umaban_sel)
            .next()
            .and_then(|c| cell_text(&c.text().collect::<String>()))
            .and_then(|t| t.parse::<u32>().ok())
            .and_then(|n| HorseNum::try_from(n).ok())
        else {
            continue;
        };
        let Some(horse_name) = extract_horse_name(&row, &horse_link_sel) else {
            continue;
        };
        let jockey = extract_jockey(&row, &jockey_sel);

        entries.push(FetchedEntry {
            gate_num,
            horse_num,
            horse_name,
            jockey,
        });
    }

    if entries.is_empty() {
        return Err(Error::Parse(
            "出馬表テーブルから出走馬を抽出できませんでした".to_string(),
        ));
    }
    Ok(entries)
}

/// `td.HorseInfo a[href*="/horse/"]` の `title` 属性から馬名を取る。
fn extract_horse_name(row: &ElementRef, sel: &Selector) -> Option<HorseName> {
    let link = row.select(sel).next()?;
    link.value()
        .attr("title")
        .and_then(cell_text)
        .and_then(|n| HorseName::try_from(n).ok())
}

/// `td.Jockey a` の `title` 属性から騎手名を取る。取れなければ None。
fn extract_jockey(row: &ElementRef, sel: &Selector) -> Option<JockeyName> {
    let link = row.select(sel).next()?;
    link.value()
        .attr("title")
        .and_then(cell_text)
        .and_then(|n| JockeyName::try_from(n).ok())
}

fn sel(s: &str) -> Result<Selector> {
    Selector::parse(s).map_err(|e| Error::Parse(format!("invalid selector {s}: {e}")))
}
