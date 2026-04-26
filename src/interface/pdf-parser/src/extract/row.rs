use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Default, Clone)]
pub struct RawRow {
    pub gate: Option<u32>,
    pub horse_num: Option<u32>,
    pub horse_name: Option<String>,
    pub jockey: Option<String>,
    pub trainer: Option<String>,
    pub time_str: Option<String>,
    /// `〃` appears in the time column when the horse finished in the same time as the previous
    /// finisher. The numeric time itself is omitted, so callers should fall back to inheriting
    /// the previous result's `time_seconds`.
    pub time_inherits: bool,
    pub margin: Option<String>,
    pub odds: Option<f64>,
    pub horse_weight: Option<u32>,
    pub weight_change: Option<i32>,
}

// Compile-once regex literals. Compiling inside hot loops (per-chunk × per-line) was wasting
// thousands of allocations per ingest.
static GATE_HORSE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:([1-8])\s+(\d{1,2})|([1-8])(\d{2}))(.?)").unwrap());
static SEX_MARKER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([牡牝セ\u{FFFD}])(\d{1,2})[^\d]").unwrap());
static GATE_HORSE_PREFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[1-8]\s?\d{1,2}\s*").unwrap());
static TIME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([1-9])[:：](\d{2})[.．](\d)").unwrap());
static BODY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<bw>\d{3})\s*(?P<sign>[＋＋\+\-－±―])\s*(?P<delta>\d{1,2})").unwrap()
});
static WEIGHT_ONLY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?P<bw>\d{3})\s*$").unwrap());
static SIGN_DELTA_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?P<sign>[＋＋\+\-－±―])\s*(?P<delta>\d{1,2})?").unwrap());
static ODDS_INLINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(\d{1,4})[.．](\d{1,2})").unwrap());
static ODDS_INT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(\d{1,4})[.．]\s*$").unwrap());
static ODDS_FRAC_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(\d{1,2})").unwrap());
static ODDS_LOOSE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{1,4})[.．](\d{1,2})").unwrap());
static JOCKEY_WEIGHT_ONLY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{1,2}$").unwrap());
static JOCKEY_MARKER_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[▲△☆◇]").unwrap());
static SEX_INLINE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[牡牝セ]\d").unwrap());

/// Detect a "gate horse_num" row-start line. Two distinct shapes:
///   "5 9"                             — gate + space + 1-2 digit horse_num
///   "815"                             — gate glued with 2-digit horse_num (no space)
///   "812�インジケーター牡5芦53"       — gate-horse glued with name (still 2-digit horse)
///   "5 5 �イプシロンナンバー..."        — spaced gate-horse plus name on same line
/// Single-digit-no-space (e.g. "55" = weight 55kg, "52" = apprentice weight) is rejected because
/// it cannot be distinguished from a real gate-horse pair without forcing the unspaced form to
/// always be 2-digit horse_num.
/// Also reject when the trailing character looks like body-weight or odds context.
pub fn parse_gate_horse_line(line: &str) -> Option<(u32, u32)> {
    let trimmed = line.trim();
    let c = GATE_HORSE_RE.captures(trimmed)?;
    let (g, h): (u32, u32) = if let Some(g1) = c.get(1) {
        (g1.as_str().parse().ok()?, c.get(2)?.as_str().parse().ok()?)
    } else {
        (
            c.get(3)?.as_str().parse().ok()?,
            c.get(4)?.as_str().parse().ok()?,
        )
    };
    if !(1..=18).contains(&h) {
        return None;
    }
    if let Some(next) = c.get(5).map(|m| m.as_str()).and_then(|s| s.chars().next())
        && (next.is_ascii_digit()
            || matches!(
                next,
                '±' | '＋' | '－' | '+' | '-' | '―' | '．' | '.' | '：' | ':' | '、' | ','
            ))
    {
        return None;
    }
    Some((g, h))
}

const RESULT_END_MARKERS: &[&str] = &[
    "売得金",
    "払戻金",
    "ハロンタイム",
    "通過タイム",
    "コーナー",
    "市場取引馬",
    "勝馬の",
    "票数",
    "上り",
];

/// Slice each "horse chunk" out of the full block of lines.
/// A chunk starts at a "gate horse_num" line and ends right before the next such line
/// (or at any of the post-results section markers).
pub fn collect_chunks(lines: &[String]) -> Vec<Vec<String>> {
    let mut chunks = Vec::new();
    let mut current: Option<Vec<String>> = None;

    for line in lines.iter() {
        let trimmed = line.trim();
        if RESULT_END_MARKERS.iter().any(|m| trimmed.starts_with(m)) {
            if let Some(c) = current.take() {
                chunks.push(c);
            }
            // Once we hit a results-end marker, stop collecting more chunks for this race.
            return chunks;
        }
        if parse_gate_horse_line(trimmed).is_some() {
            if let Some(c) = current.take() {
                chunks.push(c);
            }
            current = Some(vec![line.clone()]);
        } else if let Some(c) = current.as_mut() {
            c.push(line.clone());
        }
    }
    if let Some(c) = current.take() {
        chunks.push(c);
    }
    chunks
}

pub fn parse_chunk(chunk: &[String]) -> RawRow {
    let mut row = RawRow::default();
    if chunk.is_empty() {
        return row;
    }
    if let Some((g, h)) = parse_gate_horse_line(chunk[0].trim()) {
        row.gate = Some(g);
        row.horse_num = Some(h);
    }

    extract_horse_name(chunk, &mut row);
    let time_match_in_line = extract_time(chunk, &mut row);
    let body_pos = extract_body_weight(chunk, time_match_in_line, &mut row);
    extract_margin(chunk, &mut row);
    detect_time_inheritance(chunk, &mut row);
    extract_odds(chunk, time_match_in_line, body_pos, &mut row);

    if let Some(bw_pos) = body_pos
        && let Some(j) = guess_jockey(&chunk[..bw_pos])
    {
        row.jockey = Some(j);
    }
    if let Some(bw_pos) = body_pos
        && let Some(t) = guess_trainer(&chunk[..bw_pos])
    {
        row.trainer = Some(t);
    }

    row
}

/// Horse name + sex + age + color appear together but layouts vary widely:
///   chunk[1]    = "ロードトライデント牡3栗"          (single line)
///   chunk[0]    = "812�インジケーター牡5芦53"        (name glued onto first line)
///   chunk[1]    = "ルーラーリッチ�6鹿54"             (sex marker missing → U+FFFD)
///   chunk[1..5] = "オ" "オ" "タ" "チ牡4黒鹿57"       (one-char-per-line vertical)
/// We accumulate text until a sex marker (牡/牝/セ/U+FFFD) is found, then take everything before it.
fn extract_horse_name(chunk: &[String], row: &mut RawRow) {
    let mut name_buf = String::new();
    for (i, line) in chunk.iter().take(8).enumerate() {
        let candidate: String = if i == 0 {
            GATE_HORSE_PREFIX_RE.replace(line.trim(), "").to_string()
        } else {
            line.trim().to_string()
        };
        if let Some(c) = SEX_MARKER_RE.captures(&candidate) {
            let marker_byte_pos = c
                .get(1)
                .expect("SEX_MARKER_RE always captures group 1")
                .start();
            name_buf.push_str(&candidate[..marker_byte_pos]);
            break;
        } else {
            name_buf.push_str(&candidate);
        }
    }
    let cleaned: String = name_buf
        .chars()
        .skip_while(|c| {
            c.is_control()
                || *c == '\u{FFFD}'
                || *c == '\u{3000}'
                || ('\u{0080}'..='\u{00ff}').contains(c)
        })
        .collect();
    let cleaned = cleaned.trim().to_string();
    if !cleaned.is_empty() {
        row.horse_name = Some(cleaned);
    }
}

/// Time format "M:SS.f" / "M：SS．f". Race times are 1-9 minutes, 2-digit seconds, 1-digit frac.
/// The 1-digit minutes constraint prevents "61：11．6" from matching as 61 minutes
/// when the body-weight delta and time are jammed together (e.g. "B 478－61：11．6").
/// Returns `(chunk_idx, byte_start, byte_end)` of the matched time so callers can carve up
/// the same line for body-weight scanning.
fn extract_time(chunk: &[String], row: &mut RawRow) -> Option<(usize, usize, usize)> {
    for (i, line) in chunk.iter().enumerate() {
        if let Some(c) = TIME_RE.captures(line) {
            let start = c.get(1).expect("TIME_RE always captures group 1").start();
            let end = c.get(3).expect("TIME_RE always captures group 3").end();
            row.time_str = Some(format!("{}:{}.{}", &c[1], &c[2], &c[3]));
            return Some((i, start, end));
        }
    }
    None
}

/// Horse body weight (3-digit) and weight change ([＋−±―] N) on a single line — e.g.
///   "B 478－61：11．6"  →  478, -6   (split out the time first; parse "B 478－6" only)
///   "474± 01：13．1 クビ" →  474,  0
///   "490－6 （競走除外）" →  490, -6
/// Falls back to a 2-line layout (`488\n―1：57．9`) if no single-line match is found.
fn extract_body_weight(
    chunk: &[String],
    time_match_in_line: Option<(usize, usize, usize)>,
    row: &mut RawRow,
) -> Option<usize> {
    for (i, line) in chunk.iter().enumerate() {
        let scan_target: &str = match time_match_in_line {
            Some((ti, pos, _)) if ti == i => &line[..pos],
            _ => line.as_str(),
        };
        if let Some(c) = BODY_RE.captures(scan_target) {
            let bw: u32 = c
                .name("bw")
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
            let sign = c
                .name("sign")
                .expect("BODY_RE captures sign on match")
                .as_str();
            let delta: i32 = c
                .name("delta")
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
            row.horse_weight = Some(bw);
            row.weight_change = Some(signed_delta(sign, delta));
            return Some(i);
        }
    }
    // Fallback: weight on its own line, sign on the next line (e.g. "488\n―1：57．9").
    // The next line's leading sign — typically `―` or `±` for "no change" — collides
    // visually with the time's leading minute digit, so we don't try to read a delta value
    // when a time follows on the same line; we just take the sign and let `signed_delta` decide.
    for i in 0..chunk.len().saturating_sub(1) {
        let Some(wc) = WEIGHT_ONLY_RE.captures(chunk[i].trim()) else {
            continue;
        };
        let Some(sc) = SIGN_DELTA_RE.captures(&chunk[i + 1]) else {
            continue;
        };
        let bw: u32 = wc["bw"].parse().unwrap_or(0);
        let sign = sc
            .name("sign")
            .expect("SIGN_DELTA_RE captures sign on match")
            .as_str();
        let next_has_time = matches!(time_match_in_line, Some((ti, _, _)) if ti == i + 1);
        let delta: i32 = if next_has_time {
            0
        } else {
            sc.name("delta")
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0)
        };
        row.horse_weight = Some(bw);
        row.weight_change = Some(signed_delta(sign, delta));
        return Some(i);
    }
    None
}

/// `〃` is NOT a margin keyword — it appears in the time column for same-time horses
/// and is handled separately via `time_inherits`.
fn extract_margin(chunk: &[String], row: &mut RawRow) {
    let margin_keywords = ["ハナ", "アタマ", "クビ"];
    for line in chunk.iter() {
        if let Some(kw) = margin_keywords.iter().find(|kw| line.contains(*kw)) {
            row.margin = Some(kw.to_string());
            return;
        }
    }
}

/// Time inheritance marker: a line that is exactly `〃` means "same time as the previous
/// finisher". The numeric time is omitted in the source PDF.
fn detect_time_inheritance(chunk: &[String], row: &mut RawRow) {
    if row.time_str.is_none() && chunk.iter().any(|l| l.trim() == "〃") {
        row.time_inherits = true;
    }
}

/// Odds: scan lines after the result-table anchor (whichever of body weight / time appears).
/// Same-tied horses ("〃") have no time line, so anchor to body weight in that case.
fn extract_odds(
    chunk: &[String],
    time_match_in_line: Option<(usize, usize, usize)>,
    body_pos: Option<usize>,
    row: &mut RawRow,
) {
    let anchor_pos = match (time_match_in_line, body_pos) {
        (Some((ti, ..)), Some(bi)) => Some(ti.max(bi)),
        (Some((ti, ..)), None) => Some(ti),
        (None, Some(bi)) => Some(bi),
        (None, None) => None,
    };
    if let Some(pos) = anchor_pos {
        let tail = &chunk[pos.saturating_add(1)..];
        row.odds = parse_odds_fragments(tail);
        if row.odds.is_none()
            && let Some((ti, _, time_end)) = time_match_in_line
            && ti == pos
        {
            let after_time = &chunk[ti][time_end..];
            row.odds = parse_inline_odds(after_time);
        }
    }
}

fn signed_delta(sign: &str, delta: i32) -> i32 {
    match sign {
        "＋" | "+" => delta,
        "−" | "－" | "-" => -delta,
        "±" | "―" => 0,
        _ => 0,
    }
}

fn parse_odds_fragments(tail: &[String]) -> Option<f64> {
    let mut int_part: Option<u32> = None;
    for line in tail.iter() {
        let trimmed = line.trim();
        if let Some(c) = ODDS_INLINE_RE.captures(trimmed) {
            let ip: u32 = c[1].parse().ok()?;
            let frac_str = c.get(2)?.as_str();
            let frac: u32 = frac_str.parse().ok()?;
            return Some(ip as f64 + (frac as f64) / 10f64.powi(frac_str.len() as i32));
        }
        if let Some(c) = ODDS_INT_RE.captures(trimmed) {
            int_part = c[1].parse().ok();
            continue;
        }
        if let Some(ip) = int_part
            && let Some(c) = ODDS_FRAC_RE.captures(trimmed)
        {
            let frac: u32 = c[1].parse().ok()?;
            return Some(ip as f64 + (frac as f64) / 10f64.powi(c[1].len() as i32));
        }
    }
    None
}

fn parse_inline_odds(line: &str) -> Option<f64> {
    ODDS_LOOSE_RE.captures(line).and_then(|c| {
        let int_part: u32 = c[1].parse().ok()?;
        let frac_str = c.get(2)?.as_str();
        let frac: u32 = frac_str.parse().ok()?;
        Some(int_part as f64 + (frac as f64) / 10f64.powi(frac_str.len() as i32))
    })
}

/// Heuristic jockey extraction:
/// - skip the gate/horse_num line and the name line
/// - skip the weight line (1-2 digit number on its own)
/// - skip apprentice marker lines
/// - join the next 1-2 lines that look like Japanese name fragments
fn guess_jockey(lines_before_bw: &[String]) -> Option<String> {
    let mut buf: Vec<String> = Vec::new();
    let mut started = false;
    for line in lines_before_bw.iter().skip(1) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if !started {
            if SEX_INLINE_RE.is_match(t) {
                continue;
            }
            if JOCKEY_WEIGHT_ONLY_RE.is_match(t) {
                continue;
            }
            // Apprentice weight reduction (1-2 digit on its own) appears as another weight line.
            // Marker like ▲ may be on its own or attached to first name fragment.
            started = true;
        }
        let stripped = JOCKEY_MARKER_RE.replace(t, "").to_string();
        if !stripped.is_empty() {
            buf.push(stripped);
        }
        if buf.len() >= 2 {
            break;
        }
    }
    if buf.is_empty() {
        return None;
    }
    let joined = buf.join("");
    // The second name fragment may have owner text glued onto it (e.g. "和生�ロード...").
    // Truncate at the first owner-section marker character.
    let cleaned: String = joined
        .chars()
        .take_while(|c| !is_owner_marker(*c))
        .collect();
    Some(cleaned.trim().to_string()).filter(|s| !s.is_empty())
}

fn is_owner_marker(c: char) -> bool {
    matches!(c, '\u{0080}'..='\u{009F}') || c == '\u{FFFD}' || c == '氏' || c == '\u{3000}'
}

/// Heuristic trainer extraction: look for the line containing "氏" or a known region keyword
/// and return the substring after the owner block.
fn guess_trainer(_lines_before_bw: &[String]) -> Option<String> {
    // Trainer parsing is too noisy in this layout. Leave as None for now.
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> String {
        v.to_string()
    }

    #[test]
    fn gate_horse_spaced_pair() {
        assert_eq!(parse_gate_horse_line("5 9"), Some((5, 9)));
        assert_eq!(parse_gate_horse_line("3 12"), Some((3, 12)));
    }

    #[test]
    fn gate_horse_glued_two_digit() {
        assert_eq!(parse_gate_horse_line("815"), Some((8, 15)));
        assert_eq!(parse_gate_horse_line("712"), Some((7, 12)));
    }

    #[test]
    fn gate_horse_glued_with_name() {
        assert_eq!(
            parse_gate_horse_line("812�インジケーター牡5芦53"),
            Some((8, 12))
        );
    }

    #[test]
    fn rejects_carried_weight_only() {
        // 55 / 52 = カラ斤量、出馬投票時の値や減量騎手用の分。chunk start にしてはいけない。
        assert_eq!(parse_gate_horse_line("55"), None);
        assert_eq!(parse_gate_horse_line("52"), None);
    }

    #[test]
    fn rejects_body_weight_lines() {
        assert_eq!(parse_gate_horse_line("B 478－61：11．6"), None);
        assert_eq!(parse_gate_horse_line("474± 01：13．1 クビ"), None);
        assert_eq!(parse_gate_horse_line("448－41：13．4"), None);
    }

    #[test]
    fn rejects_odds_lines() {
        assert_eq!(parse_gate_horse_line("1．7"), None);
        assert_eq!(parse_gate_horse_line("151．"), None);
        assert_eq!(parse_gate_horse_line("4．2�"), None);
    }

    #[test]
    fn rejects_horse_num_out_of_range() {
        // Horse num > 18 with glued form. "999" gives gate=9 which is itself out of [1-8] — rejected.
        assert_eq!(parse_gate_horse_line("999"), None);
        // Horse num 19 (out of range) glued → rejected by the (1..=18) check.
        assert_eq!(parse_gate_horse_line("819"), None);
    }

    #[test]
    fn signed_delta_signs() {
        assert_eq!(signed_delta("＋", 5), 5);
        assert_eq!(signed_delta("+", 5), 5);
        assert_eq!(signed_delta("－", 5), -5);
        assert_eq!(signed_delta("-", 5), -5);
        assert_eq!(signed_delta("±", 5), 0);
        assert_eq!(signed_delta("―", 5), 0);
    }

    #[test]
    fn odds_split_two_lines() {
        let tail = vec![s("1．"), s("7�")];
        assert_eq!(parse_odds_fragments(&tail), Some(1.7));
    }

    #[test]
    fn odds_split_three_digits() {
        let tail = vec![s("100．"), s("7�")];
        assert_eq!(parse_odds_fragments(&tail), Some(100.7));
    }

    #[test]
    fn odds_inline_single_line() {
        let tail = vec![s("4．2�")];
        assert_eq!(parse_odds_fragments(&tail), Some(4.2));
    }

    #[test]
    fn odds_inline_three_digit() {
        let tail = vec![s("192．5")];
        assert_eq!(parse_odds_fragments(&tail), Some(192.5));
    }

    #[test]
    fn time_extraction_from_jammed_body_line() {
        // body+time in one line — the time regex should yank "1：11．6" and not "61：11．6".
        let chunk = vec![
            s("5 9"),
            s("ロードトライデント牡3栗"),
            s("B 478－61：11．6"),
        ];
        let row = parse_chunk(&chunk);
        assert_eq!(row.time_str.as_deref(), Some("1:11.6"));
        assert_eq!(row.horse_weight, Some(478));
        assert_eq!(row.weight_change, Some(-6));
    }

    #[test]
    fn ditto_marks_time_inherits_flag() {
        let chunk = vec![
            s("2 2"),
            s("クィーンコッチャン牝3鹿"),
            s("452－6"),
            s("〃"),
            s("ハナ"),
        ];
        let row = parse_chunk(&chunk);
        assert!(row.time_inherits, "expected time_inherits=true");
        assert!(
            row.time_str.is_none(),
            "expected time_str=None when only 〃 is present"
        );
        assert_eq!(row.horse_weight, Some(452));
        assert_eq!(row.weight_change, Some(-6));
        assert_eq!(row.margin.as_deref(), Some("ハナ"));
    }

    #[test]
    fn weight_split_across_two_lines() {
        // weight on its own line, sign+time on the next — fallback path.
        let chunk = vec![
            s("5 9"),
            s("ブルーフレア牡3青鹿57"),
            s("488"),
            s("―1：57．9 2"),
        ];
        let row = parse_chunk(&chunk);
        assert_eq!(row.horse_weight, Some(488));
        assert_eq!(row.weight_change, Some(0)); // ― なので 0
        assert_eq!(row.time_str.as_deref(), Some("1:57.9"));
    }
}
