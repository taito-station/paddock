mod stext;

use paddock_domain::{GateNum, HorseEntry, HorseName, HorseNum, JockeyName, RaceCard, RaceId, Surface, Venue};
use regex::Regex;

use crate::error::{Error, Result};
use stext::StextDoc;

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
                l.page == origin.page
                    && l.x >= origin.col_x - 20.0
                    && l.x <= origin.col_x + 290.0
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
        .filter(|l| l.size >= 17.0 && l.y >= 55.0 && l.y <= 110.0)
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

    // --- header (y < origin.race_num_y + 60) ---
    let header_end_y = lines
        .iter()
        .find(|l| l.size >= 17.0 && (l.x - cx).abs() < 5.0)
        .map(|l| l.y + 60.0)
        .unwrap_or(140.0);

    let header_lines: Vec<&FlatLine> = lines.iter().copied().filter(|l| l.y < header_end_y).collect();
    let entry_lines: Vec<&FlatLine> = lines.iter().copied().filter(|l| l.y >= header_end_y).collect();

    let (year, round, venue, day) = match extract_meeting(&header_lines) {
        Some(v) => v,
        None => return Ok(None),
    };
    let distance = match extract_distance(&header_lines) {
        Some(v) => v,
        None => return Ok(None),
    };
    let surface = match extract_surface(&header_lines) {
        Some(v) => v,
        None => return Ok(None),
    };

    let race_id_str = format!("{}-{}-{}-{}-{}R", year, round, venue.as_slug(), day, origin.race_num);
    let race_id = RaceId::try_from(race_id_str)?;

    let raw_entries = extract_entries(&entry_lines, cx);
    let mut entries = Vec::with_capacity(raw_entries.len());
    for raw in raw_entries {
        let gate_num = GateNum::try_from(raw.gate_num)?;
        let horse_num = HorseNum::try_from(raw.horse_num)?;
        let horse_name = HorseName::try_from(raw.horse_name.as_str())?;
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

/// Extract `（year, round, venue, day）` from the compact header text `2026年3中山8`.
fn extract_meeting(lines: &[&FlatLine]) -> Option<(i32, u32, Venue, u32)> {
    let re = Regex::new(
        r"(?P<y>\d{4})年(?P<r>\d+)(?P<v>札幌|函館|福島|新潟|東京|中山|中京|京都|阪神|小倉)(?P<d>\d+)",
    )
    .unwrap();
    for l in lines {
        if let Some(cap) = re.captures(&l.text) {
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
        if let Ok(n) = normalized.parse::<u32>() {
            if (800..=4000).contains(&n) {
                return Some(n);
            }
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

        // Gate color (白/黒/赤/青/黄/緑/橙/桃), size ≈ 7–9, offset ≈ -5..12
        if l.size >= 7.0 && l.size <= 9.5 && (-5.0..=12.0).contains(&off) {
            if let Some(&(_, gate)) = GATE_COLORS.iter().find(|(s, _)| *s == l.text.trim()) {
                gate_events.push((l.y, gate));
                continue;
            }
        }

        // Horse num: size ≈ 9–11 (GI full-field races use size=9), offset ∈ [5, 28], digit 1–18
        if l.size >= 8.5 && l.size <= 12.5 && (5.0..=28.0).contains(&off) {
            if let Ok(n) = l.text.trim().parse::<u32>() {
                if (1..=18).contains(&n) {
                    horse_num_events.push((l.y, n));
                    continue;
                }
            }
        }

        // Name fragment: size ≈ 9–11 (GI uses size=9), offset ∈ [25, 155), non-ASCII present.
        // Horse names start at offset ≈33–34; jockeys start at offset ≈160.
        // The [25, 155) bound keeps them separate. Gate-related text ("発走" etc.) is
        // at size ≤ 8 and thus excluded.
        if l.size >= 8.5 && l.size <= 12.5 && (25.0..155.0).contains(&off) {
            if l.text.chars().any(|c| !c.is_ascii()) {
                name_fragments.push((l.y, l.x, l.text.clone()));
                continue;
            }
        }

        // Jockey: any size 9–11, offset ∈ [155, 235]
        if l.size >= 9.0 && l.size <= 11.5 && (155.0..=235.0).contains(&off) {
            if l.text.chars().any(|c| !c.is_ascii()) {
                jockey_fragments.push((l.y, l.x, l.text.clone()));
            }
        }
    }

    gate_events.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    horse_num_events.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    // Build horse names: group name_fragments by y (bucket to nearest 3 units)
    // then concatenate fragments sorted by x.
    let horse_names = group_by_y(&name_fragments, 3.0);

    // Build jockey names similarly.
    let jockey_map = group_by_y(&jockey_fragments, 8.0);

    // Match names to horse nums sequentially by y-rank (i-th name ↔ i-th num).
    let n = horse_names.len().min(horse_num_events.len());
    let mut entries = Vec::with_capacity(n);

    for i in 0..n {
        let (name_y, horse_name) = &horse_names[i];
        let (num_y, horse_num_val) = horse_num_events[i];

        // Gate = latest gate event at y ≤ max(name_y, num_y)
        let boundary_y = name_y.max(num_y);
        let gate_num = gate_events
            .iter()
            .filter(|(gy, _)| *gy <= boundary_y + 5.0)
            .last()
            .map(|(_, g)| *g)
            .unwrap_or(1);

        // Jockey: look for jockey_map entry whose y is within ±10 of name_y
        let jockey = find_jockey(&jockey_map, *name_y, 12.0);

        entries.push(RawEntry {
            gate_num,
            horse_num: horse_num_val,
            horse_name: horse_name.clone(),
            jockey,
        });
    }

    entries
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

fn find_jockey(map: &[(f64, String)], name_y: f64, tolerance: f64) -> Option<String> {
    map.iter()
        .filter(|(jy, _)| (*jy - name_y).abs() <= tolerance)
        .min_by(|a, b| {
            (a.0 - name_y)
                .abs()
                .partial_cmp(&(b.0 - name_y).abs())
                .unwrap()
        })
        .map(|(_, t)| t.clone())
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
        let lines = vec![FlatLine { page: 0, x: 229.0, y: 49.0, size: 6.0, text: "1，800".into() }];
        let refs: Vec<&FlatLine> = lines.iter().collect();
        assert_eq!(extract_distance(&refs), Some(1800));
    }

    #[test]
    fn extract_distance_ascii_comma() {
        let lines = vec![FlatLine { page: 0, x: 229.0, y: 49.0, size: 6.0, text: "1,200".into() }];
        let refs: Vec<&FlatLine> = lines.iter().collect();
        assert_eq!(extract_distance(&refs), Some(1200));
    }

    #[test]
    fn extract_surface_dirt() {
        let lines = vec![FlatLine { page: 0, x: 234.0, y: 56.0, size: 6.0, text: "（ダート".into() }];
        let refs: Vec<&FlatLine> = lines.iter().collect();
        assert_eq!(extract_surface(&refs), Some(Surface::Dirt));
    }

    #[test]
    fn extract_surface_turf() {
        let lines = vec![FlatLine { page: 0, x: 1106.0, y: 56.0, size: 6.0, text: "（芝".into() }];
        let refs: Vec<&FlatLine> = lines.iter().collect();
        assert_eq!(extract_surface(&refs), Some(Surface::Turf));
    }

    #[test]
    fn extract_meeting_compact_header() {
        let lines = vec![FlatLine { page: 0, x: 30.0, y: 49.0, size: 5.0, text: "2026年3中山8".into() }];
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
