use regex::Regex;

#[derive(Debug, Default, Clone)]
pub struct RawRow {
    pub gate: Option<u32>,
    pub horse_num: Option<u32>,
    pub horse_name: Option<String>,
    pub jockey: Option<String>,
    pub trainer: Option<String>,
    pub time_str: Option<String>,
    pub margin: Option<String>,
    pub odds: Option<f64>,
    pub horse_weight: Option<u32>,
    pub weight_change: Option<i32>,
}

/// Detect a "gate horse_num" row-start line. Several variants:
///   "5 9"                             — pure pair
///   "815"                             — glued (gate=8, horse=15)
///   "812�インジケーター牡5芦53"       — gate-horse glued with name
///   "5 5 �イプシロンナンバー..."        — spaced gate-horse plus name on same line
/// Reject lines that look like body-weight rows (e.g. "518± 0...") or odds (e.g. "1．7"):
/// the next character after the captured horse_num must not be another digit or a sign.
pub fn parse_gate_horse_line(line: &str) -> Option<(u32, u32)> {
    let trimmed = line.trim();
    let re = Regex::new(r"^([1-8])\s?(\d{1,2})(.?)").unwrap();
    let c = re.captures(trimmed)?;
    let g: u32 = c[1].parse().ok()?;
    let h: u32 = c[2].parse().ok()?;
    if !(1..=18).contains(&h) {
        return None;
    }
    if let Some(next) = c.get(3).map(|m| m.as_str()).and_then(|s| s.chars().next()) {
        if next.is_ascii_digit()
            || matches!(
                next,
                '±' | '＋' | '－' | '+' | '-' | '―' | '．' | '.' | '：' | ':' | '、' | ','
            )
        {
            return None;
        }
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

    // Horse name + sex + age + color appears together, but the layout has many variants:
    //   chunk[1] = "ロードトライデント牡3栗"                     (clean single line)
    //   chunk[0] = "812�インジケーター牡5芦53"                   (name glued onto first line)
    //   chunk[1] = "ルーラーリッチ�6鹿54"                        (sex marker missing → U+FFFD)
    //   chunk[1..5] = "オ" "オ" "タ" "チ牡4黒鹿57"               (one-char-per-line vertical)
    // Strategy: starting at the first line (with the gate prefix stripped), accumulate text
    // line by line until a sex marker (牡/牝/セ/U+FFFD) is found, then take everything before
    // that marker as the name.
    let sex_marker_re = Regex::new(r"([牡牝セ\u{FFFD}])(\d{1,2})[^\d]").unwrap();
    let prefix_re = Regex::new(r"^[1-8]\s?\d{1,2}\s*").unwrap();
    let mut name_buf = String::new();
    for (i, line) in chunk.iter().take(8).enumerate() {
        let candidate: String = if i == 0 {
            prefix_re.replace(line.trim(), "").to_string()
        } else {
            line.trim().to_string()
        };
        if let Some(c) = sex_marker_re.captures(&candidate) {
            let marker_byte_pos = c.get(1).unwrap().start();
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

    // Time: looking for "M:SS.f" or "M：SS．f" pattern, anywhere in chunk.
    let time_re = Regex::new(r"(\d{1,2}[:：]\d{1,2}[.．]\d{1,2})").unwrap();
    for line in chunk.iter() {
        if let Some(c) = time_re.captures(line) {
            row.time_str = Some(c[1].to_string());
            break;
        }
    }

    // Horse body weight (3-digit) and weight change ([＋−±―] N) on a single line — e.g.
    //   "B 478－61：11．6"  →  478, -6
    //   "474± 01：13．1 クビ" →  474,  0
    //   "490－6 （競走除外）" →  490, -6
    let body_re =
        Regex::new(r"(?P<bw>\d{3})\s*(?P<sign>[＋＋\+\-－±―])\s*(?P<delta>\d{1,3})").unwrap();
    for line in chunk.iter() {
        if let Some(c) = body_re.captures(line) {
            row.horse_weight = c.name("bw").and_then(|m| m.as_str().parse().ok());
            let sign = c.name("sign").unwrap().as_str();
            let delta: i32 = c
                .name("delta")
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
            row.weight_change = Some(match sign {
                "＋" | "+" => delta,
                "−" | "－" | "-" => -delta,
                "±" | "―" => 0,
                _ => 0,
            });
            break;
        }
    }

    // Margin keywords or short numeric on the time line trailing part.
    let margin_keywords = ["ハナ", "アタマ", "クビ", "〃"];
    for line in chunk.iter() {
        if let Some(kw) = margin_keywords.iter().find(|kw| line.contains(*kw)) {
            row.margin = Some(kw.to_string());
            break;
        }
    }

    // Odds: scan lines after the time for fragments like "1．" / "7"  → 1.7
    //                                                or "192．" / "5"  → 192.5
    if let Some(time_pos) = chunk.iter().position(|l| {
        Regex::new(r"\d{1,2}[:：]\d{1,2}[.．]\d{1,2}")
            .unwrap()
            .is_match(l)
    }) {
        let tail = &chunk[time_pos.saturating_add(1)..];
        row.odds = parse_odds_fragments(tail);
        // Sometimes odds appear on the same line as the time.
        if row.odds.is_none() {
            row.odds = parse_inline_odds(&chunk[time_pos]);
        }
    }

    // Jockey name: try the line(s) just before the body-weight line.
    // Skip any apprentice marker (▲ △ ☆ ◇).
    if let Some(bw_pos) = chunk
        .iter()
        .position(|l| Regex::new(r"\d{3}\s*[＋＋\+\-－±―]").unwrap().is_match(l))
    {
        if let Some(j) = guess_jockey(&chunk[..bw_pos]) {
            row.jockey = Some(j);
        }
        if let Some(t) = guess_trainer(&chunk[..bw_pos]) {
            row.trainer = Some(t);
        }
    }

    row
}

fn parse_odds_fragments(tail: &[String]) -> Option<f64> {
    let int_re = Regex::new(r"^\s*(\d{1,4})[.．]\s*$").unwrap();
    let frac_re = Regex::new(r"^\s*(\d{1,2})").unwrap();
    let mut int_part: Option<u32> = None;
    for line in tail.iter() {
        let trimmed = line.trim();
        if let Some(c) = int_re.captures(trimmed) {
            int_part = c[1].parse().ok();
            continue;
        }
        if let Some(ip) = int_part {
            if let Some(c) = frac_re.captures(trimmed) {
                let frac: u32 = c[1].parse().ok()?;
                return Some(ip as f64 + (frac as f64) / 10f64.powi(c[1].len() as i32));
            }
        }
    }
    None
}

fn parse_inline_odds(line: &str) -> Option<f64> {
    let re = Regex::new(r"(\d{1,4})[.．](\d{1,2})").unwrap();
    re.captures(line).and_then(|c| {
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
    let weight_only = Regex::new(r"^\d{1,2}$").unwrap();
    let marker_re = Regex::new(r"^[▲△☆◇]").unwrap();
    let sex_re = Regex::new(r"[牡牝セ]\d").unwrap();
    let mut buf: Vec<String> = Vec::new();
    let mut started = false;
    for line in lines_before_bw.iter().skip(1) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if !started {
            if sex_re.is_match(t) {
                continue;
            }
            if weight_only.is_match(t) {
                continue;
            }
            // Apprentice weight reduction (1-2 digit on its own) appears as another weight line.
            // Marker like ▲ may be on its own or attached to first name fragment.
            started = true;
        }
        let stripped = marker_re.replace(t, "").to_string();
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
