mod stext;

use std::sync::LazyLock;

use paddock_domain::{
    GateNum, HorseEntry, HorseName, HorseNum, JockeyName, RaceCard, RaceId, Surface, Venue,
};
use regex::Regex;

use crate::error::{Error, Result};
use stext::StextDoc;

// ── layout thresholds ───────────────────────────────────────────────────────
// The JRA race-card PDF packs 4 races per page. Within a race column, fields are
// identified by font size and x-offset from the column origin (the large race-number
// glyph). These constants capture the empirically observed bands.

/// Race-number glyph size used to locate each race column.
const RACE_NUM_MIN_SIZE: f64 = 17.0;
/// Gate-color kanji (白/黒/…) font-size band and x-offset band.
const GATE_SIZE: std::ops::RangeInclusive<f64> = 7.0..=9.5;
const GATE_OFFSET: std::ops::RangeInclusive<f64> = -5.0..=12.0;
/// Horse-number / horse-name font-size band (GI full-field races shrink to size 9).
const ROW_SIZE: std::ops::RangeInclusive<f64> = 8.5..=12.5;
const HORSE_NUM_OFFSET: std::ops::RangeInclusive<f64> = 5.0..=28.0;
/// Horse names start at offset ≈33 and end well before the jockey column (≈160).
/// The 150 upper bound also excludes the "発走" (post-time) label at offset ≈154.
const NAME_OFFSET: std::ops::Range<f64> = 25.0..150.0;
/// Jockey font-size band and x-offset band.
const JOCKEY_SIZE: std::ops::RangeInclusive<f64> = 9.0..=11.5;
const JOCKEY_OFFSET: std::ops::RangeInclusive<f64> = 155.0..=235.0;

/// A horse number sits ≈13 units below its name; the row pitch is ≈47. A tolerance
/// between those two keeps each number bound to its own name and avoids stealing a
/// neighbouring row's name when one is missing.
const NAME_NUM_TOLERANCE: f64 = 25.0;
/// Jockey text sits almost level with the horse name.
const JOCKEY_TOLERANCE: f64 = 12.0;
/// y-bucket sizes for consolidating split glyphs onto a single logical line.
const NAME_BUCKET: f64 = 3.0;
const JOCKEY_BUCKET: f64 = 8.0;

/// A flattened text line with its page index.
struct FlatLine {
    page: usize,
    x: f64,
    y: f64,
    size: f64,
    text: String,
}

/// Location of a single race column on a page.
struct RaceOrigin {
    page: usize,
    col_x: f64,
    race_num: u32,
}

/// Raw extracted data for a single horse entry before domain type conversion.
struct RawEntry {
    gate_num: u32,
    horse_num: u32,
    horse_name: String,
    jockey: Option<String>,
}

const GATE_COLORS: &[(&str, u32)] = &[
    ("白", 1),
    ("黒", 2),
    ("赤", 3),
    ("青", 4),
    ("黄", 5),
    ("緑", 6),
    ("橙", 7),
    ("桃", 8),
];

pub fn parse_stext(json: &str) -> Result<Vec<RaceCard>> {
    let doc: StextDoc =
        serde_json::from_str(json).map_err(|e| Error::Parse(format!("stext.json: {e}")))?;

    let all_lines = flatten(&doc);
    let origins = find_race_origins(&all_lines);

    let mut cards = Vec::new();
    for origin in &origins {
        let col_lines: Vec<&FlatLine> = all_lines
            .iter()
            .filter(|l| {
                l.page == origin.page && l.x >= origin.col_x - 20.0 && l.x <= origin.col_x + 290.0
            })
            .collect();

        if let Some(card) = parse_column(&col_lines, origin)? {
            cards.push(card);
        }
    }
    Ok(cards)
}

fn flatten(doc: &StextDoc) -> Vec<FlatLine> {
    let mut out = Vec::new();
    for (page, p) in doc.pages.iter().enumerate() {
        for block in &p.blocks {
            for line in &block.lines {
                out.push(FlatLine {
                    page,
                    x: line.x,
                    y: line.y,
                    size: line.font.size,
                    text: line.text.clone(),
                });
            }
        }
    }
    out
}

/// Race column origins are identified by the large race-number text (size ≥ 17,
/// text = "1"–"12") near y ∈ [60, 100].
fn find_race_origins(lines: &[FlatLine]) -> Vec<RaceOrigin> {
    let mut origins: Vec<RaceOrigin> = lines
        .iter()
        .filter(|l| l.size >= RACE_NUM_MIN_SIZE && l.y >= 55.0 && l.y <= 110.0)
        .filter_map(|l| {
            let n: u32 = l.text.trim().parse().ok()?;
            if (1..=12).contains(&n) {
                Some(RaceOrigin {
                    page: l.page,
                    col_x: l.x,
                    race_num: n,
                })
            } else {
                None
            }
        })
        .collect();
    origins.sort_by(|a, b| {
        a.page
            .cmp(&b.page)
            .then(a.col_x.partial_cmp(&b.col_x).unwrap())
    });
    origins
}

fn parse_column(lines: &[&FlatLine], origin: &RaceOrigin) -> Result<Option<RaceCard>> {
    let cx = origin.col_x;

    // --- header (y < race-number glyph y + 60) ---
    let header_end_y = lines
        .iter()
        .find(|l| l.size >= RACE_NUM_MIN_SIZE && (l.x - cx).abs() < 5.0)
        .map(|l| l.y + 60.0)
        .unwrap_or(140.0);

    let header_lines: Vec<&FlatLine> = lines
        .iter()
        .copied()
        .filter(|l| l.y < header_end_y)
        .collect();
    let entry_lines: Vec<&FlatLine> = lines
        .iter()
        .copied()
        .filter(|l| l.y >= header_end_y)
        .collect();

    let Some((year, round, venue, day)) = extract_meeting(&header_lines) else {
        tracing::warn!(
            race_num = origin.race_num,
            "race-card meeting header not parsed, skipping"
        );
        return Ok(None);
    };
    let Some(distance) = extract_distance(&header_lines) else {
        tracing::warn!(
            race_num = origin.race_num,
            "race-card distance not parsed, skipping"
        );
        return Ok(None);
    };
    let Some(surface) = extract_surface(&header_lines) else {
        tracing::warn!(
            race_num = origin.race_num,
            "race-card surface not parsed, skipping"
        );
        return Ok(None);
    };

    let race_id_str = format!(
        "{}-{}-{}-{}-{}R",
        year,
        round,
        venue.as_slug(),
        day,
        origin.race_num
    );
    let race_id = RaceId::try_from(race_id_str)?;

    let raw_entries = extract_entries(&entry_lines, cx);
    let mut entries = Vec::with_capacity(raw_entries.len());
    for raw in raw_entries {
        // A single malformed row should not abort the whole PDF: skip it (with a warning)
        // so the remaining races/entries still get ingested.
        let (Ok(gate_num), Ok(horse_num)) = (
            GateNum::try_from(raw.gate_num),
            HorseNum::try_from(raw.horse_num),
        ) else {
            tracing::warn!(
                gate = raw.gate_num,
                horse_num = raw.horse_num,
                "invalid gate/horse number, skipping entry"
            );
            continue;
        };
        let horse_name = match HorseName::try_from(raw.horse_name.as_str()) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(name = %raw.horse_name, "invalid horse name ({e}), skipping entry");
                continue;
            }
        };
        let jockey = raw
            .jockey
            .filter(|j| !j.is_empty())
            .and_then(|j| JockeyName::try_from(j.as_str()).ok());
        entries.push(HorseEntry {
            gate_num,
            horse_num,
            horse_name,
            jockey,
        });
    }

    Ok(Some(RaceCard {
        race_id,
        venue,
        round,
        day,
        race_num: origin.race_num,
        surface,
        distance,
        entries,
    }))
}

// ── header field extractors ───────────────────────────────────────────────────

static MEETING_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?P<y>\d{4})年(?P<r>\d+)(?P<v>札幌|函館|福島|新潟|東京|中山|中京|京都|阪神|小倉)(?P<d>\d+)",
    )
    .unwrap()
});

/// Extract `（year, round, venue, day）` from the compact header text `2026年3中山8`.
fn extract_meeting(lines: &[&FlatLine]) -> Option<(i32, u32, Venue, u32)> {
    for l in lines {
        if let Some(cap) = MEETING_RE.captures(&l.text) {
            let year: i32 = cap.name("y")?.as_str().parse().ok()?;
            let round: u32 = cap.name("r")?.as_str().parse().ok()?;
            let venue = Venue::try_from(cap.name("v")?.as_str()).ok()?;
            let day: u32 = cap.name("d")?.as_str().parse().ok()?;
            return Some((year, round, venue, day));
        }
    }
    None
}

/// Extract distance from text like `1，200` (fullwidth comma) or `1,200`.
fn extract_distance(lines: &[&FlatLine]) -> Option<u32> {
    for l in lines {
        let digits: String = l
            .text
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '，' || *c == ',')
            .collect();
        let normalized = digits.replace(['，', ','], "");
        if normalized.is_empty() {
            continue;
        }
        if let Ok(n) = normalized.parse::<u32>()
            && (800..=4000).contains(&n)
        {
            return Some(n);
        }
    }
    None
}

/// Extract surface from `（ダート` → Dirt or `（芝` → Turf.
fn extract_surface(lines: &[&FlatLine]) -> Option<Surface> {
    for l in lines {
        if l.text.contains("ダート") || l.text.contains("（ダ") {
            return Some(Surface::Dirt);
        }
        if l.text.contains("（芝") {
            return Some(Surface::Turf);
        }
    }
    None
}

// ── horse entry extractor ─────────────────────────────────────────────────────

fn extract_entries(lines: &[&FlatLine], col_x: f64) -> Vec<RawEntry> {
    // Classify lines by their x-offset from the column origin.
    // offset = line.x - col_x

    // Gate color markers: single kanji, size ≈ 8, offset ∈ [-10, 12]
    let mut gate_events: Vec<(f64, u32)> = Vec::new(); // (y, gate_num)

    // Horse nums: size ≈ 11, offset ∈ [5, 28], digit 1–18
    let mut horse_num_events: Vec<(f64, u32)> = Vec::new(); // (y, num)

    // Name fragments: size ≈ 11, offset ∈ [25, 160], at least one non-ASCII char
    // Group by (y rounded to nearest 2) to consolidate split chars on the same line.
    let mut name_fragments: Vec<(f64, f64, String)> = Vec::new(); // (y, x, text)

    // Jockey fragments: size ≈ 10, offset ∈ [148, 230]
    let mut jockey_fragments: Vec<(f64, f64, String)> = Vec::new(); // (y, x, text)

    for l in lines {
        let off = l.x - col_x;

        // Gate color (白/黒/赤/青/黄/緑/橙/桃).
        if GATE_SIZE.contains(&l.size)
            && GATE_OFFSET.contains(&off)
            && let Some(&(_, gate)) = GATE_COLORS.iter().find(|(s, _)| *s == l.text.trim())
        {
            gate_events.push((l.y, gate));
            continue;
        }

        // Horse num: digit 1–18 in the number column.
        if ROW_SIZE.contains(&l.size)
            && HORSE_NUM_OFFSET.contains(&off)
            && let Ok(n) = l.text.trim().parse::<u32>()
            && (1..=18).contains(&n)
        {
            horse_num_events.push((l.y, n));
            continue;
        }

        // Name fragment: non-ASCII text in the name column. The NAME_OFFSET upper bound
        // keeps jockey text (which starts ≈160) out; gate text ("発走" etc.) is size ≤ 8.
        if ROW_SIZE.contains(&l.size) && NAME_OFFSET.contains(&off) && !l.text.is_ascii() {
            name_fragments.push((l.y, l.x, l.text.clone()));
            continue;
        }

        // Jockey fragment.
        if JOCKEY_SIZE.contains(&l.size) && JOCKEY_OFFSET.contains(&off) && !l.text.is_ascii() {
            jockey_fragments.push((l.y, l.x, l.text.clone()));
        }
    }

    gate_events.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    horse_num_events.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    // Consolidate split glyphs into one logical line each.
    let horse_names = group_by_y(&name_fragments, NAME_BUCKET);
    let jockey_map = group_by_y(&jockey_fragments, JOCKEY_BUCKET);

    // Anchor on horse names (the essential field) and bind each to the nearest horse number
    // by y. This makes each row independent, so a single missing/extra glyph no longer
    // cascades into a name↔number misalignment across the whole column.
    let mut entries = Vec::with_capacity(horse_names.len());
    for (name_y, horse_name) in &horse_names {
        let Some(&(num_y, horse_num_val)) = nearest(&horse_num_events, *name_y, NAME_NUM_TOLERANCE)
        else {
            tracing::warn!(name = %horse_name, "no horse number near name, skipping entry");
            continue;
        };

        // Gate = latest gate event at or above this row.
        let boundary_y = name_y.max(num_y);
        let gate_num = gate_events
            .iter()
            .rfind(|(gy, _)| *gy <= boundary_y + 5.0)
            .map(|(_, g)| *g)
            .unwrap_or(1);

        let jockey = nearest(&jockey_map, *name_y, JOCKEY_TOLERANCE).map(|(_, t)| t.clone());

        entries.push(RawEntry {
            gate_num,
            horse_num: horse_num_val,
            horse_name: horse_name.clone(),
            jockey,
        });
    }

    entries
}

/// Return the element whose `y` (the `.0` field) is closest to `target`, within `tolerance`.
fn nearest<T>(items: &[(f64, T)], target: f64, tolerance: f64) -> Option<&(f64, T)> {
    items
        .iter()
        .filter(|(y, _)| (*y - target).abs() <= tolerance)
        .min_by(|a, b| {
            (a.0 - target)
                .abs()
                .partial_cmp(&(b.0 - target).abs())
                .unwrap()
        })
}

/// Group `(y, x, text)` fragments: snap y to the nearest `bucket` units, then
/// within each y-bucket sort by x and concatenate texts.
/// Returns a list of `(representative_y, concatenated_text)` sorted by y.
fn group_by_y(fragments: &[(f64, f64, String)], bucket: f64) -> Vec<(f64, String)> {
    if fragments.is_empty() {
        return Vec::new();
    }

    // Collect raw y values
    let mut ys: Vec<f64> = fragments.iter().map(|(y, _, _)| *y).collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ys.dedup_by(|a, b| (*a - *b).abs() <= bucket);

    let mut groups: Vec<(f64, String)> = ys
        .iter()
        .map(|&gy| {
            let mut parts: Vec<(f64, &str)> = fragments
                .iter()
                .filter(|(y, _, _)| (*y - gy).abs() <= bucket)
                .map(|(_, x, t)| (*x, t.as_str()))
                .collect();
            parts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            let text = parts.into_iter().map(|(_, t)| t).collect::<String>();
            (gy, text)
        })
        .filter(|(_, t)| !t.is_empty())
        .collect();

    groups.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    groups
}

// ── unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_color_map_covers_all_eight_gates() {
        let gates: Vec<u32> = GATE_COLORS.iter().map(|(_, g)| *g).collect();
        assert_eq!(gates, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn extract_distance_fullwidth_comma() {
        let lines = vec![FlatLine {
            page: 0,
            x: 229.0,
            y: 49.0,
            size: 6.0,
            text: "1，800".into(),
        }];
        let refs: Vec<&FlatLine> = lines.iter().collect();
        assert_eq!(extract_distance(&refs), Some(1800));
    }

    #[test]
    fn extract_distance_ascii_comma() {
        let lines = vec![FlatLine {
            page: 0,
            x: 229.0,
            y: 49.0,
            size: 6.0,
            text: "1,200".into(),
        }];
        let refs: Vec<&FlatLine> = lines.iter().collect();
        assert_eq!(extract_distance(&refs), Some(1200));
    }

    #[test]
    fn extract_surface_dirt() {
        let lines = vec![FlatLine {
            page: 0,
            x: 234.0,
            y: 56.0,
            size: 6.0,
            text: "（ダート".into(),
        }];
        let refs: Vec<&FlatLine> = lines.iter().collect();
        assert_eq!(extract_surface(&refs), Some(Surface::Dirt));
    }

    #[test]
    fn extract_surface_turf() {
        let lines = vec![FlatLine {
            page: 0,
            x: 1106.0,
            y: 56.0,
            size: 6.0,
            text: "（芝".into(),
        }];
        let refs: Vec<&FlatLine> = lines.iter().collect();
        assert_eq!(extract_surface(&refs), Some(Surface::Turf));
    }

    #[test]
    fn extract_meeting_compact_header() {
        let lines = vec![FlatLine {
            page: 0,
            x: 30.0,
            y: 49.0,
            size: 5.0,
            text: "2026年3中山8".into(),
        }];
        let refs: Vec<&FlatLine> = lines.iter().collect();
        let (year, round, venue, day) = extract_meeting(&refs).expect("should parse");
        assert_eq!(year, 2026);
        assert_eq!(round, 3);
        assert_eq!(venue, Venue::Nakayama);
        assert_eq!(day, 8);
    }

    #[test]
    fn group_by_y_concatenates_name_chars() {
        let frags = vec![
            (138.0, 69.0, "ノ".to_string()),
            (138.0, 94.0, "ー".to_string()),
            (138.5, 120.0, "チ".to_string()),
        ];
        let result = group_by_y(&frags, 3.0);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, "ノーチ");
    }
}
