use std::collections::HashMap;

use crate::extract::columns::cluster_into_rows;
use crate::tesseract::OcrToken;

/// One row of OCR-recovered fields keyed by horse number.
#[derive(Debug, Clone, Default)]
pub struct OcrRow {
    pub horse_num: u32,
    pub finishing_position: Option<u32>,
    pub trainer: Option<String>,
    pub weight_carried: Option<f64>,
    pub popularity: Option<u32>,
}

/// All rows recovered from a single PDF page.
#[derive(Debug, Clone, Default)]
pub struct OcrPageRows {
    pub page_num: u32,
    pub rows: Vec<OcrRow>,
}

/// Convert raw OCR tokens for a page into rows keyed by horse number.
/// This is intentionally a heuristic skeleton: it detects table rows by clustering tokens
/// vertically, then for each row picks out plausible (horse_num, finishing_position, weight,
/// popularity, trainer) fields by their relative x position. Refinement happens incrementally
/// as we observe layout variants.
pub fn tokens_to_rows(page_num: u32, tokens: &[OcrToken]) -> OcrPageRows {
    let visual_rows = cluster_into_rows(tokens);
    let mut rows = Vec::new();
    for visual_row in visual_rows {
        if let Some(row) = extract_horse_row(&visual_row) {
            rows.push(row);
        }
    }
    let merged = merge_duplicates(rows);
    OcrPageRows {
        page_num,
        rows: merged,
    }
}

fn extract_horse_row(tokens: &[OcrToken]) -> Option<OcrRow> {
    // The first one or two integer tokens are usually finishing_position and gate; the next
    // small integer is horse_num. We treat horse_num as the first 2-digit integer in 1..=18,
    // and finishing_position as the first 1-2 digit integer earlier in the row that is in 1..=18.
    let ints: Vec<(usize, u32)> = tokens
        .iter()
        .enumerate()
        .filter_map(|(i, t)| t.text.parse::<u32>().ok().map(|n| (i, n)))
        .collect();
    if ints.is_empty() {
        return None;
    }

    let horse_num_pair = ints.iter().find(|(_, n)| (1..=18).contains(n)).copied()?;
    let mut row = OcrRow {
        horse_num: horse_num_pair.1,
        ..Default::default()
    };

    // Finishing position: an integer in 1..=18 that appears strictly before horse_num.
    if let Some((_, pos)) = ints
        .iter()
        .find(|(i, n)| *i < horse_num_pair.0 && (1..=18).contains(n))
        .copied()
    {
        row.finishing_position = Some(pos);
    }

    // Carried weight: a small decimal or 2-digit integer between roughly 48..=64 kg.
    for t in tokens {
        if let Ok(n) = t.text.parse::<f64>()
            && (48.0..=64.0).contains(&n)
        {
            row.weight_carried = Some(n);
            break;
        }
    }

    // Popularity: any 1-2 digit integer occurring after horse_num that is in 1..=18 and is
    // not the same field as horse_num.
    for (i, n) in &ints {
        if *i > horse_num_pair.0 && (1..=18).contains(n) {
            row.popularity = Some(*n);
            break;
        }
    }

    Some(row)
}

fn merge_duplicates(rows: Vec<OcrRow>) -> Vec<OcrRow> {
    let mut by_num: HashMap<u32, OcrRow> = HashMap::new();
    for r in rows {
        let entry = by_num.entry(r.horse_num).or_insert_with(|| OcrRow {
            horse_num: r.horse_num,
            ..Default::default()
        });
        if entry.finishing_position.is_none() {
            entry.finishing_position = r.finishing_position;
        }
        if entry.trainer.is_none() {
            entry.trainer = r.trainer;
        }
        if entry.weight_carried.is_none() {
            entry.weight_carried = r.weight_carried;
        }
        if entry.popularity.is_none() {
            entry.popularity = r.popularity;
        }
    }
    let mut out: Vec<OcrRow> = by_num.into_values().collect();
    out.sort_by_key(|r| r.horse_num);
    out
}
