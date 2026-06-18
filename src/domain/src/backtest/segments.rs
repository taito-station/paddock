//! 人気帯・頭数帯・馬場（芝/ダート）別のセグメント集計と、その分類バンド定数。

use std::collections::HashMap;

use super::metrics::calibration;
use super::model::{
    FieldSizeSegment, HorseOutcome, PopularitySegment, RaceEvaluation, SurfaceSegment,
};
use crate::Surface;

/// 人気帯セグメントのラベル（出力順）。`popularity_band` の戻り値と一致させる。
pub(crate) const POPULARITY_BANDS: [&str; 6] = [
    "1番人気",
    "2-3番人気",
    "4-6番人気",
    "7-9番人気",
    "10番人気以下",
    "人気不明",
];

/// 頭数帯セグメントのラベル（出力順）。`field_size_band` の戻り値と一致させる。
pub(crate) const FIELD_SIZE_BANDS: [&str; 4] = ["～9頭", "10-12頭", "13-15頭", "16頭以上"];

/// 馬場（芝/ダート）セグメントのラベル（出力順）。`surface_band` の戻り値と一致させる。
pub(crate) const SURFACE_BANDS: [&str; 2] = ["芝", "ダート"];

/// 人気を人気帯ラベルへ分類する。`None` は「人気不明」。
pub(crate) fn popularity_band(popularity: Option<u32>) -> &'static str {
    match popularity {
        Some(1) => "1番人気",
        Some(2..=3) => "2-3番人気",
        Some(4..=6) => "4-6番人気",
        Some(7..=9) => "7-9番人気",
        Some(_) => "10番人気以下",
        None => "人気不明",
    }
}

/// 出走頭数を頭数帯ラベルへ分類する。
pub(crate) fn field_size_band(field_size: usize) -> &'static str {
    match field_size {
        0..=9 => "～9頭",
        10..=12 => "10-12頭",
        13..=15 => "13-15頭",
        _ => "16頭以上",
    }
}

/// 馬場を馬場ラベルへ分類する。
pub(crate) fn surface_band(surface: Surface) -> &'static str {
    match surface {
        Surface::Turf => "芝",
        Surface::Dirt => "ダート",
    }
}

/// 人気帯別の集計（馬エントリ単位）。データのある帯のみ [`POPULARITY_BANDS`] 順に返す。
pub(crate) fn popularity_segments(races: &[RaceEvaluation]) -> Vec<PopularitySegment> {
    let mut buckets: HashMap<&'static str, Vec<&HorseOutcome>> = HashMap::new();
    for race in races {
        for h in &race.horses {
            buckets
                .entry(popularity_band(h.popularity))
                .or_default()
                .push(h);
        }
    }

    POPULARITY_BANDS
        .iter()
        .filter_map(|&label| {
            // バケットは `or_default().push()` でしか作られないため、キーがあれば必ず非空。
            let horses = buckets.get(label)?;
            let entries = horses.len() as u32;
            let pairs: Vec<(f64, bool)> = horses.iter().map(|h| (h.win_prob, h.won())).collect();
            let mean_win_prob = horses.iter().map(|h| h.win_prob).sum::<f64>() / entries as f64;
            let observed_win_rate =
                horses.iter().filter(|h| h.won()).count() as f64 / entries as f64;
            Some(PopularitySegment {
                label: label.to_string(),
                entries,
                mean_win_prob,
                observed_win_rate,
                win_calibration: calibration(&pairs),
            })
        })
        .collect()
}

/// 頭数帯別の集計（レース単位）。データのある帯のみ [`FIELD_SIZE_BANDS`] 順に返す。
pub(crate) fn field_size_segments(races: &[RaceEvaluation]) -> Vec<FieldSizeSegment> {
    let mut buckets: HashMap<&'static str, Vec<&RaceEvaluation>> = HashMap::new();
    for race in races {
        buckets
            .entry(field_size_band(race.field_size()))
            .or_default()
            .push(race);
    }

    FIELD_SIZE_BANDS
        .iter()
        .filter_map(|&label| {
            // バケットは `or_default().push()` でしか作られないため、キーがあれば必ず非空。
            let group = buckets.get(label)?;
            let races_n = group.len() as f64;
            let mut win_hits = 0u32;
            let mut place_hits = 0u32;
            let mut show_hits = 0u32;
            let mut pairs: Vec<(f64, bool)> = Vec::new();
            for race in group {
                if let Some(pos) = race.top_pick_position {
                    if pos == 1 {
                        win_hits += 1;
                    }
                    if pos <= 2 {
                        place_hits += 1;
                    }
                    if pos <= 3 {
                        show_hits += 1;
                    }
                }
                for h in &race.horses {
                    pairs.push((h.win_prob, h.won()));
                }
            }
            Some(FieldSizeSegment {
                label: label.to_string(),
                races: group.len() as u32,
                win_hit_rate: win_hits as f64 / races_n,
                place_hit_rate: place_hits as f64 / races_n,
                show_hit_rate: show_hits as f64 / races_n,
                win_calibration: calibration(&pairs),
            })
        })
        .collect()
}

/// 馬場（芝/ダート）別の集計（レース単位）。データのある馬場のみ [`SURFACE_BANDS`] 順に返す。
/// `field_size_segments` と同じ集計方式（トップ選好馬の着順で的中、全馬エントリで単勝校正）。
pub(crate) fn surface_segments(races: &[RaceEvaluation]) -> Vec<SurfaceSegment> {
    let mut buckets: HashMap<&'static str, Vec<&RaceEvaluation>> = HashMap::new();
    for race in races {
        buckets
            .entry(surface_band(race.surface))
            .or_default()
            .push(race);
    }

    SURFACE_BANDS
        .iter()
        .filter_map(|&label| {
            // バケットは `or_default().push()` でしか作られないため、キーがあれば必ず非空。
            let group = buckets.get(label)?;
            let races_n = group.len() as f64;
            let mut win_hits = 0u32;
            let mut place_hits = 0u32;
            let mut show_hits = 0u32;
            let mut pairs: Vec<(f64, bool)> = Vec::new();
            for race in group {
                if let Some(pos) = race.top_pick_position {
                    if pos == 1 {
                        win_hits += 1;
                    }
                    if pos <= 2 {
                        place_hits += 1;
                    }
                    if pos <= 3 {
                        show_hits += 1;
                    }
                }
                for h in &race.horses {
                    pairs.push((h.win_prob, h.won()));
                }
            }
            Some(SurfaceSegment {
                label: label.to_string(),
                races: group.len() as u32,
                win_hit_rate: win_hits as f64 / races_n,
                place_hit_rate: place_hits as f64 / races_n,
                show_hit_rate: show_hits as f64 / races_n,
                win_calibration: calibration(&pairs),
            })
        })
        .collect()
}
