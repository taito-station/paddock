use crate::extract::columns::cluster_into_rows;
use crate::tesseract::OcrToken;

/// One row of OCR-recovered fields. `horse_name` is the matching key used by the merge layer
/// (different races repeat horse_num 1..18 within the same PDF, so a numeric key alone is
/// ambiguous).
#[derive(Debug, Clone, Default)]
pub struct OcrRow {
    pub horse_num: u32,
    pub horse_name: Option<String>,
    pub finishing_position: Option<u32>,
    pub trainer: Option<String>,
    pub weight_carried: Option<f64>,
    pub popularity: Option<u32>,
}

/// All rows recovered from a single PDF page. The page index is preserved so callers can
/// distinguish duplicates that come from different races on different pages.
#[derive(Debug, Clone, Default)]
pub struct OcrPageRows {
    pub page_num: u32,
    pub rows: Vec<OcrRow>,
}

/// Convert raw OCR tokens for a page into rows.
/// Heuristic: cluster tokens by y coordinate into visual rows, then per row pick out
/// horse_num, optional finishing_position, weight_carried, popularity, and a katakana
/// horse_name candidate. Rows from different races are kept separate (no horse_num dedup
/// across pages — each race's horse_num space is independent).
pub fn tokens_to_rows(page_num: u32, tokens: &[OcrToken]) -> OcrPageRows {
    let visual_rows = cluster_into_rows(tokens);
    let mut rows = Vec::new();
    for visual_row in visual_rows {
        if let Some(row) = extract_horse_row(&visual_row) {
            rows.push(row);
        }
    }
    OcrPageRows { page_num, rows }
}

fn extract_horse_row(tokens: &[OcrToken]) -> Option<OcrRow> {
    let ints: Vec<(usize, u32)> = tokens
        .iter()
        .enumerate()
        .filter_map(|(i, t)| t.text.parse::<u32>().ok().map(|n| (i, n)))
        .collect();
    if ints.is_empty() {
        return None;
    }

    // horse_num: first 1-18 integer in the row.
    let horse_num_pair = ints.iter().find(|(_, n)| (1..=18).contains(n)).copied()?;
    let mut row = OcrRow {
        horse_num: horse_num_pair.1,
        ..Default::default()
    };

    // finishing_position: an integer in 1..=18 that appears strictly before horse_num.
    if let Some((_, pos)) = ints
        .iter()
        .find(|(i, n)| *i < horse_num_pair.0 && (1..=18).contains(n))
        .copied()
    {
        row.finishing_position = Some(pos);
    }

    // weight_carried (斤量): JRA は 48.0〜63.5kg、半 kg 刻み。
    // 半端値 (e.g. "52.5") を優先し、なければ整数値で 48..=63 の範囲を採る。
    let mut fractional: Option<f64> = None;
    let mut integral: Option<f64> = None;
    for t in tokens {
        if let Ok(n) = t.text.parse::<f64>() {
            if (48.0..=63.5).contains(&n) && n.fract() != 0.0 && fractional.is_none() {
                fractional = Some(n);
            } else if (48.0..=63.0).contains(&n) && n.fract() == 0.0 && integral.is_none() {
                integral = Some(n);
            }
        }
    }
    row.weight_carried = fractional.or(integral);

    // popularity: integer after horse_num in 1..=18.
    for (i, n) in &ints {
        if *i > horse_num_pair.0 && (1..=18).contains(n) {
            row.popularity = Some(*n);
            break;
        }
    }

    // horse_name candidate: longest contiguous run of katakana-heavy tokens.
    row.horse_name = extract_name_candidate(tokens);

    Some(row)
}

/// Take adjacent tokens whose text is mostly katakana (≥75%) and join them. JRA horse names
/// are written in katakana so this is a high-precision filter.
fn extract_name_candidate(tokens: &[OcrToken]) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    let mut current = String::new();
    for t in tokens {
        if is_katakana_dominant(&t.text) {
            current.push_str(&t.text);
        } else if !current.is_empty() {
            if best
                .as_ref()
                .is_none_or(|(len, _)| current.chars().count() > *len)
            {
                best = Some((current.chars().count(), std::mem::take(&mut current)));
            }
            current.clear();
        }
    }
    if !current.is_empty()
        && best
            .as_ref()
            .is_none_or(|(len, _)| current.chars().count() > *len)
    {
        best = Some((current.chars().count(), current));
    }
    best.and_then(|(len, s)| if len >= 3 { Some(s) } else { None })
}

fn is_katakana_dominant(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 2 {
        return false;
    }
    let kata_count = chars
        .iter()
        .filter(|c| ('\u{30A0}'..='\u{30FF}').contains(c))
        .count();
    kata_count * 4 >= chars.len() * 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn katakana_dominant_passes_horse_names() {
        assert!(is_katakana_dominant("ロードトライデント"));
        assert!(is_katakana_dominant("プラチナムディスク"));
        // 1 char is too short.
        assert!(!is_katakana_dominant("ロ"));
        // Mostly digits → not a name.
        assert!(!is_katakana_dominant("478"));
        // Kanji → not a katakana name.
        assert!(!is_katakana_dominant("中山競馬"));
    }
}
