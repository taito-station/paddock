use chrono::NaiveDate;
use paddock_domain::{Surface, TrackCondition, Venue, Weather};
use regex::Regex;

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct RaceHeader {
    pub date: NaiveDate,
    pub year: i32,
    pub round: u32,
    pub venue: Venue,
    pub day: u32,
    pub race_num: u32,
    pub surface: Surface,
    pub distance: u32,
    pub track_condition: Option<TrackCondition>,
    pub weather: Option<Weather>,
}

/// Race start lines look like `"08061 4月12日晴"` (5-digit race code + date + weather).
pub fn is_race_start_line(line: &str) -> bool {
    let trimmed = line.trim();
    let re = Regex::new(r"^\d{5}\s+\d+月\d+日\s*[晴曇雨雪]").unwrap();
    re.is_match(trimmed)
}

pub fn parse_header(lines: &[String]) -> Result<Option<RaceHeader>> {
    if lines.is_empty() {
        return Ok(None);
    }
    let head = &lines[0];
    let date_re =
        Regex::new(r"^(?P<code>\d{5})\s+(?P<m>\d+)月(?P<d>\d+)日\s*(?P<w>[晴曇雨雪小]+)").unwrap();
    let cap = match date_re.captures(head) {
        Some(c) => c,
        None => return Ok(None),
    };
    let month: u32 = cap.name("m").unwrap().as_str().parse().unwrap_or(0);
    let day_of_month: u32 = cap.name("d").unwrap().as_str().parse().unwrap_or(0);
    let weather_str = cap.name("w").unwrap().as_str();
    let weather = Weather::try_from(weather_str).ok();

    let mut year: Option<i32> = None;
    let mut round: Option<u32> = None;
    let mut venue: Option<Venue> = None;
    let mut day: Option<u32> = None;
    let mut race_num: Option<u32> = None;
    let mut surface: Option<Surface> = None;
    let mut distance: Option<u32> = None;
    let mut track_condition: Option<TrackCondition> = None;

    let meeting_re = Regex::new(
        r"（(?P<y>\d{4})年(?P<r>\d+)(?P<v>札幌|函館|福島|新潟|東京|中山|中京|京都|阪神|小倉)）第(?P<d>\d+)日",
    )
    .unwrap();
    let race_num_re = Regex::new(r"第(?P<n>\d+)競走").unwrap();
    let surface_re = Regex::new(r"（(?P<s>芝|ダート)").unwrap();
    let condition_re = Regex::new(r"^\s*(良|稍重|稍|重|不良|不)\s*$").unwrap();

    // Scan ~30 header lines for the various fields.
    let scan_limit = lines.len().min(40);
    for line in &lines[..scan_limit] {
        if year.is_none()
            && let Some(c) = meeting_re.captures(line)
        {
            year = c.name("y").unwrap().as_str().parse().ok();
            round = c.name("r").unwrap().as_str().parse().ok();
            venue = Venue::try_from(c.name("v").unwrap().as_str()).ok();
            day = c.name("d").unwrap().as_str().parse().ok();
        }
        if race_num.is_none()
            && let Some(c) = race_num_re.captures(line)
        {
            race_num = c.name("n").unwrap().as_str().parse().ok();
        }
        if surface.is_none()
            && let Some(c) = surface_re.captures(line)
        {
            surface = Surface::try_from(c.name("s").unwrap().as_str()).ok();
        }
        if distance.is_none() {
            if let Some(d) = parse_distance(line) {
                distance = Some(d);
            }
        }
        if track_condition.is_none() {
            if let Some(c) = condition_re.captures(line.trim()) {
                track_condition = TrackCondition::try_from(c.get(1).unwrap().as_str()).ok();
            }
        }
    }

    // Track condition often appears on the second line.
    if track_condition.is_none() && lines.len() >= 2 {
        let l2 = lines[1].trim();
        track_condition = TrackCondition::try_from(l2).ok();
    }

    let (
        Some(year),
        Some(round),
        Some(venue),
        Some(day),
        Some(race_num),
        Some(surface),
        Some(distance),
    ) = (year, round, venue, day, race_num, surface, distance)
    else {
        return Ok(None);
    };
    let date = NaiveDate::from_ymd_opt(year, month, day_of_month).ok_or_else(|| {
        crate::error::Error::Parse(format!("invalid date {year}-{month:02}-{day_of_month:02}"))
    })?;
    Ok(Some(RaceHeader {
        date,
        year,
        round,
        venue,
        day,
        race_num,
        surface,
        distance,
        track_condition,
        weather,
    }))
}

/// Distance lines look like `"��1，200�"` or `"1，800m"` — strip non-digit/comma chars
/// and parse a number in 800..=4000.
fn parse_distance(line: &str) -> Option<u32> {
    let only_digits: String = line
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '，' || *c == ',')
        .collect();
    let normalized = only_digits.replace('，', "").replace(',', "");
    if normalized.is_empty() {
        return None;
    }
    let n: u32 = normalized.parse().ok()?;
    if (800..=4000).contains(&n) {
        Some(n)
    } else {
        None
    }
}

/// Find the `（N頭）` line and return N.
pub fn find_field_size(lines: &[String]) -> Option<u32> {
    let re = Regex::new(r"（(?P<n>\d+)頭）").unwrap();
    lines.iter().find_map(|l| {
        re.captures(l)
            .and_then(|c| c.name("n")?.as_str().parse().ok())
    })
}
