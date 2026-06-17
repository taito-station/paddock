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
///
/// The whitespace between the 5-digit code and the date is optional: 2025
/// autumn (Oct–Dec) result PDFs omit the space entirely (`"2700110月4日曇"`),
/// so the separator is `\s*`, not `\s+`. The code is fixed-width (5 digits), so
/// dropping the required space cannot let `\d{5}` bleed into the date digits.
///
/// The weather class includes `小` (`小雨`/`小雪`) to stay consistent with
/// [`parse_header`]: a race whose start line read `…日小雨` was otherwise not
/// detected as a boundary, silently merging it into the previous race's block
/// and dropping one race per meeting.
///
/// Detection only needs to recognize the *start* of the weather token, so the
/// class is a single char here (no `+`), whereas [`parse_header`] captures the
/// full weather word with `[晴曇雨雪小]+`. The asymmetry is intentional:
/// detection is lenient, parsing is precise.
pub fn is_race_start_line(line: &str) -> bool {
    let trimmed = line.trim();
    let re = Regex::new(r"^\d{5}\s*\d+月\d+日\s*[晴曇雨雪小]").unwrap();
    re.is_match(trimmed)
}

pub fn parse_header(lines: &[String]) -> Result<Option<RaceHeader>> {
    if lines.is_empty() {
        return Ok(None);
    }
    let head = &lines[0];
    // `\s*` (not `\s+`): the space between code and date is absent in 2025
    // autumn PDFs — see `is_race_start_line`.
    let date_re =
        Regex::new(r"^(?P<code>\d{5})\s*(?P<m>\d+)月(?P<d>\d+)日\s*(?P<w>[晴曇雨雪小]+)").unwrap();
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
        if distance.is_none()
            && let Some(d) = parse_distance(line)
        {
            distance = Some(d);
        }
        if track_condition.is_none()
            && let Some(c) = condition_re.captures(line.trim())
        {
            track_condition = TrackCondition::try_from(c.get(1).unwrap().as_str()).ok();
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
    let normalized = only_digits.replace(['，', ','], "");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn race_start_line_matches_with_or_without_space_after_code() {
        // Spring/normal PDFs keep a space between the 5-digit code and the date.
        assert!(is_race_start_line("14001 6月7日晴"));
        // 2025 autumn (Oct–Dec) PDFs drop the space — this is the regression
        // (issue #149): the meeting's races were dropped entirely as 0.
        assert!(is_race_start_line("2700110月4日曇"));
        // `小雨`/`小雪` weather must be detected too; otherwise that race's start
        // line is missed and the race is silently merged into the previous one.
        assert!(is_race_start_line("2701010月4日小雨"));
        // Non-header lines must still be rejected.
        assert!(!is_race_start_line("枠番馬番馬名"));
        // Adding `小` to the weather class must not create false positives: a
        // line containing `小` but lacking the code+date shape stays rejected.
        assert!(!is_race_start_line("当日は小雨"));
        // 5 digits followed by `小` but no `月日` date is not a race start.
        assert!(!is_race_start_line("12345小雨"));
    }

    #[test]
    fn parse_header_reads_spaceless_autumn_line() {
        // Minimal header block mirroring a 2025 autumn result PDF: the start
        // line has no space after the code, and the meeting line carries the
        // round/venue/day plus surface and distance.
        let lines: Vec<String> = [
            "2700110月4日曇",
            "良",
            "（2025年4東京）第1日",
            "第1競走",
            "（芝）1，600",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let header = parse_header(&lines).unwrap().expect("header should parse");
        assert_eq!(header.date, NaiveDate::from_ymd_opt(2025, 10, 4).unwrap());
        assert_eq!(header.year, 2025);
        assert_eq!(header.round, 4);
        assert_eq!(header.venue, Venue::Tokyo);
        assert_eq!(header.day, 1);
        assert_eq!(header.race_num, 1);
        assert_eq!(header.surface, Surface::Turf);
        assert_eq!(header.distance, 1600);
    }
}
