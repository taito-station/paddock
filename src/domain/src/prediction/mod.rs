use crate::horse_result::{HorseName, HorseNum};
use crate::race_card::HorseEntry;

#[derive(Debug, Clone, Copy, Default)]
pub struct RateTriple {
    pub win: f64,
    pub place: f64,
    pub show: f64,
}

#[derive(Debug, Clone)]
pub struct HorseFactors {
    pub course_gate: RateTriple,
    pub horse_surface: RateTriple,
    pub horse_distance: RateTriple,
    pub jockey_surface: Option<RateTriple>,
}

#[derive(Debug, Clone)]
pub struct HorseProbability {
    pub horse_num: HorseNum,
    pub horse_name: HorseName,
    pub win_prob: f64,
    pub place_prob: f64,
    pub show_prob: f64,
}

pub fn estimate_probabilities(entries: &[(HorseEntry, HorseFactors)]) -> Vec<HorseProbability> {
    if entries.is_empty() {
        return Vec::new();
    }

    let n = entries.len();
    let uniform = 1.0 / n as f64;

    let win_scores: Vec<f64> = entries.iter().map(|(_, f)| raw_score(f, |r| r.win)).collect();
    let place_scores: Vec<f64> = entries.iter().map(|(_, f)| raw_score(f, |r| r.place)).collect();
    let show_scores: Vec<f64> = entries.iter().map(|(_, f)| raw_score(f, |r| r.show)).collect();

    let win_probs = normalize(&win_scores, uniform);
    let place_probs = normalize(&place_scores, uniform);
    let show_probs = normalize(&show_scores, uniform);

    entries
        .iter()
        .enumerate()
        .map(|(i, (entry, _))| HorseProbability {
            horse_num: entry.horse_num,
            horse_name: entry.horse_name.clone(),
            win_prob: win_probs[i],
            place_prob: place_probs[i],
            show_prob: show_probs[i],
        })
        .collect()
}

const COURSE_GATE_WEIGHT: f64 = 2.0;

fn raw_score(factors: &HorseFactors, rate: fn(&RateTriple) -> f64) -> f64 {
    COURSE_GATE_WEIGHT * rate(&factors.course_gate)
        + rate(&factors.horse_surface)
        + rate(&factors.horse_distance)
        + factors.jockey_surface.map(|rt| rate(&rt)).unwrap_or(0.0)
}

fn normalize(scores: &[f64], fallback: f64) -> Vec<f64> {
    let total: f64 = scores.iter().sum();
    if total <= 0.0 {
        vec![fallback; scores.len()]
    } else {
        scores.iter().map(|s| s / total).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horse_result::{HorseName, HorseNum};
    use crate::race_card::HorseEntry;

    fn make_entry(horse_num: u32, horse_name: &str) -> HorseEntry {
        HorseEntry {
            gate_num: crate::horse_result::GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(horse_num).unwrap(),
            horse_name: HorseName::try_from(horse_name).unwrap(),
            jockey: None,
        }
    }

    fn zero_factors() -> HorseFactors {
        HorseFactors {
            course_gate: RateTriple::default(),
            horse_surface: RateTriple::default(),
            horse_distance: RateTriple::default(),
            jockey_surface: None,
        }
    }

    #[test]
    fn empty_entries() {
        let result = estimate_probabilities(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn uniform_fallback_when_all_scores_zero() {
        let entries = vec![
            (make_entry(1, "ウマA"), zero_factors()),
            (make_entry(2, "ウマB"), zero_factors()),
            (make_entry(3, "ウマC"), zero_factors()),
        ];
        let probs = estimate_probabilities(&entries);
        assert_eq!(probs.len(), 3);
        for p in &probs {
            assert!((p.win_prob - 1.0 / 3.0).abs() < 1e-10);
        }
        let total: f64 = probs.iter().map(|p| p.win_prob).sum();
        assert!((total - 1.0).abs() < 1e-10);
    }

    #[test]
    fn normalizes_to_one() {
        let entries = vec![
            (
                make_entry(1, "ウマA"),
                HorseFactors {
                    course_gate: RateTriple { win: 0.2, place: 0.4, show: 0.6 },
                    horse_surface: RateTriple { win: 0.1, place: 0.2, show: 0.3 },
                    horse_distance: RateTriple { win: 0.15, place: 0.3, show: 0.45 },
                    jockey_surface: None,
                },
            ),
            (
                make_entry(2, "ウマB"),
                HorseFactors {
                    course_gate: RateTriple { win: 0.1, place: 0.2, show: 0.3 },
                    horse_surface: RateTriple { win: 0.05, place: 0.1, show: 0.15 },
                    horse_distance: RateTriple { win: 0.08, place: 0.16, show: 0.24 },
                    jockey_surface: None,
                },
            ),
        ];
        let probs = estimate_probabilities(&entries);
        assert_eq!(probs.len(), 2);
        let win_total: f64 = probs.iter().map(|p| p.win_prob).sum();
        let place_total: f64 = probs.iter().map(|p| p.place_prob).sum();
        let show_total: f64 = probs.iter().map(|p| p.show_prob).sum();
        assert!((win_total - 1.0).abs() < 1e-10);
        assert!((place_total - 1.0).abs() < 1e-10);
        assert!((show_total - 1.0).abs() < 1e-10);
    }

    #[test]
    fn jockey_none_uses_zero() {
        let with_jockey = HorseFactors {
            course_gate: RateTriple { win: 0.2, place: 0.4, show: 0.6 },
            horse_surface: RateTriple::default(),
            horse_distance: RateTriple::default(),
            jockey_surface: Some(RateTriple { win: 0.1, place: 0.2, show: 0.3 }),
        };
        let without_jockey = HorseFactors {
            course_gate: RateTriple { win: 0.2, place: 0.4, show: 0.6 },
            horse_surface: RateTriple::default(),
            horse_distance: RateTriple::default(),
            jockey_surface: None,
        };
        let score_with = raw_score(&with_jockey, |r| r.win);
        let score_without = raw_score(&without_jockey, |r| r.win);
        // jockey なし → jockey_surface_rate = 0.0 として加算
        // score_without = 2.0 * 0.2 + 0.0 + 0.0 + 0.0 = 0.4
        let expected = COURSE_GATE_WEIGHT * 0.2;
        assert!((score_without - expected).abs() < 1e-10, "score_without={score_without}");
        assert!(score_with > score_without);
    }
}
