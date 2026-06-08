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

    let win_scores: Vec<f64> = entries
        .iter()
        .map(|(_, f)| raw_score(f, |r| r.win))
        .collect();
    let place_scores: Vec<f64> = entries
        .iter()
        .map(|(_, f)| raw_score(f, |r| r.place))
        .collect();
    let show_scores: Vec<f64> = entries
        .iter()
        .map(|(_, f)| raw_score(f, |r| r.show))
        .collect();

    // win は 1 着（1 ポジション）、place は 2 着以内（2 ポジション）、show は 3 着以内（3 ポジション）
    // に相当するため、レース内合計をそれぞれ 1.0 / 2.0 / 3.0 へ正規化する。各馬は確率上限 1.0。
    let win_probs = normalize_to_sum(&win_scores, 1.0);
    let mut place_probs = normalize_to_sum(&place_scores, 2.0);
    let mut show_probs = normalize_to_sum(&show_scores, 3.0);

    // 馬ごとに累積 max で単調化し win_prob ≤ place_prob ≤ show_prob を保証する。
    // win/place/show は別レートから独立に正規化するため、レート比率次第で正規化後に逆転が
    // 残りうる。これを後処理で常に是正する。
    for i in 0..place_probs.len() {
        place_probs[i] = place_probs[i].max(win_probs[i]).min(1.0);
        show_probs[i] = show_probs[i].max(place_probs[i]).min(1.0);
    }

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
const SURFACE_WEIGHT: f64 = 1.0;
const DISTANCE_WEIGHT: f64 = 1.0;
const JOCKEY_WEIGHT: f64 = 1.0;

/// 存在する factor の**重み付き平均**を返す。騎手未登録馬は jockey 項と重み（`JOCKEY_WEIGHT`）を
/// 母数から除外して評価するため、欠落項（旧実装の `+0.0`）で不当に減点されない。全馬が騎手あり
/// （または全馬なし）のときは定数除算となり、レース内正規化後の相対順位は変わらない。
fn raw_score(factors: &HorseFactors, rate: fn(&RateTriple) -> f64) -> f64 {
    let mut weighted = COURSE_GATE_WEIGHT * rate(&factors.course_gate)
        + SURFACE_WEIGHT * rate(&factors.horse_surface)
        + DISTANCE_WEIGHT * rate(&factors.horse_distance);
    let mut weight = COURSE_GATE_WEIGHT + SURFACE_WEIGHT + DISTANCE_WEIGHT;
    if let Some(jockey) = factors.jockey_surface {
        weighted += JOCKEY_WEIGHT * rate(&jockey);
        weight += JOCKEY_WEIGHT;
    }
    weighted / weight
}

/// スコアをレース内合計が `target` になるよう正規化し、各値を確率として `[0, 1]` にクランプする。
/// 全スコアが 0（出走馬全員のスタッツ未蓄積）の場合は均等フォールバック `target / n`（上限 1.0）。
fn normalize_to_sum(scores: &[f64], target: f64) -> Vec<f64> {
    let n = scores.len();
    let total: f64 = scores.iter().sum();
    if total <= 0.0 {
        let each = (target / n as f64).min(1.0);
        return vec![each; n];
    }
    scores
        .iter()
        .map(|s| (s / total * target).min(1.0))
        .collect()
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
        // win=1/3, place=2/3, show=3/3=1.0（3 頭立てなら全馬が複勝圏）。すべて単調。
        for p in &probs {
            assert!((p.win_prob - 1.0 / 3.0).abs() < 1e-10);
            assert!((p.place_prob - 2.0 / 3.0).abs() < 1e-10);
            assert!((p.show_prob - 1.0).abs() < 1e-10);
            assert!(p.win_prob <= p.place_prob && p.place_prob <= p.show_prob);
        }
        let win_total: f64 = probs.iter().map(|p| p.win_prob).sum();
        assert!((win_total - 1.0).abs() < 1e-10);
    }

    #[test]
    fn win_sums_to_one_and_values_monotone_small_field() {
        let entries = vec![
            (
                make_entry(1, "ウマA"),
                HorseFactors {
                    course_gate: RateTriple {
                        win: 0.2,
                        place: 0.4,
                        show: 0.6,
                    },
                    horse_surface: RateTriple {
                        win: 0.1,
                        place: 0.2,
                        show: 0.3,
                    },
                    horse_distance: RateTriple {
                        win: 0.15,
                        place: 0.3,
                        show: 0.45,
                    },
                    jockey_surface: None,
                },
            ),
            (
                make_entry(2, "ウマB"),
                HorseFactors {
                    course_gate: RateTriple {
                        win: 0.1,
                        place: 0.2,
                        show: 0.3,
                    },
                    horse_surface: RateTriple {
                        win: 0.05,
                        place: 0.1,
                        show: 0.15,
                    },
                    horse_distance: RateTriple {
                        win: 0.08,
                        place: 0.16,
                        show: 0.24,
                    },
                    jockey_surface: None,
                },
            ),
        ];
        let probs = estimate_probabilities(&entries);
        assert_eq!(probs.len(), 2);
        // win は 1 着＝1 ポジションなので合計 ≒ 1.0。place/show は小頭数だと上限 1.0 クランプで
        // 合計が 2/3 を下回りうるため、ここでは各値が [0,1] かつ単調であることを確認する。
        let win_total: f64 = probs.iter().map(|p| p.win_prob).sum();
        assert!((win_total - 1.0).abs() < 1e-10);
        for p in &probs {
            assert!((0.0..=1.0).contains(&p.win_prob));
            assert!((0.0..=1.0).contains(&p.place_prob));
            assert!((0.0..=1.0).contains(&p.show_prob));
            assert!(p.win_prob <= p.place_prob && p.place_prob <= p.show_prob);
        }
    }

    /// 上限クランプが起きない十分大きい均等フィールドでは place 合計 ≒ 2.0、show 合計 ≒ 3.0。
    #[test]
    fn place_show_sum_to_two_and_three_in_even_field() {
        let triple = RateTriple {
            win: 0.1,
            place: 0.2,
            show: 0.3,
        };
        let factors = HorseFactors {
            course_gate: triple,
            horse_surface: triple,
            horse_distance: triple,
            jockey_surface: None,
        };
        // 6 頭立て・全馬同一スコア → win=1/6, place=2/6, show=3/6（いずれも 1.0 未満で無クランプ）。
        let entries: Vec<_> = (1..=6)
            .map(|i| (make_entry(i, &format!("ウマ{i}")), factors.clone()))
            .collect();
        let probs = estimate_probabilities(&entries);
        let win_total: f64 = probs.iter().map(|p| p.win_prob).sum();
        let place_total: f64 = probs.iter().map(|p| p.place_prob).sum();
        let show_total: f64 = probs.iter().map(|p| p.show_prob).sum();
        assert!((win_total - 1.0).abs() < 1e-9, "win_total={win_total}");
        assert!(
            (place_total - 2.0).abs() < 1e-9,
            "place_total={place_total}"
        );
        assert!((show_total - 3.0).abs() < 1e-9, "show_total={show_total}");
    }

    /// win レートが高く place/show レートが相対的に低い馬でも、後処理の累積 max で
    /// win ≤ place ≤ show が必ず成立する。
    #[test]
    fn monotonicity_guaranteed_even_with_inverted_rates() {
        // ウマA: win 偏重（place/show が win より低い不自然なレート）。ウマB: 逆。
        let a = HorseFactors {
            course_gate: RateTriple {
                win: 0.9,
                place: 0.1,
                show: 0.1,
            },
            horse_surface: RateTriple::default(),
            horse_distance: RateTriple::default(),
            jockey_surface: None,
        };
        let b = HorseFactors {
            course_gate: RateTriple {
                win: 0.1,
                place: 0.9,
                show: 0.9,
            },
            horse_surface: RateTriple::default(),
            horse_distance: RateTriple::default(),
            jockey_surface: None,
        };
        let entries = vec![(make_entry(1, "ウマA"), a), (make_entry(2, "ウマB"), b)];
        let probs = estimate_probabilities(&entries);
        for p in &probs {
            assert!(
                p.win_prob <= p.place_prob && p.place_prob <= p.show_prob,
                "non-monotonic: {p:?}"
            );
        }
    }

    /// 列ごとに独立してフォールバック判定するため、一部の列だけ合計 0 になりうる
    /// （例: 全馬 place=show=0 だが win>0）。合計 0 の列は均等フォールバック（place→min(2/n,1)、
    /// show→min(3/n,1)）になり、累積 max により win ≤ place ≤ show は保たれる。
    #[test]
    fn monotonic_when_only_some_columns_are_all_zero() {
        // win レートのみ非ゼロ、place/show レートは全馬 0。
        let win_only = |w: f64| HorseFactors {
            course_gate: RateTriple {
                win: w,
                place: 0.0,
                show: 0.0,
            },
            horse_surface: RateTriple::default(),
            horse_distance: RateTriple::default(),
            jockey_surface: None,
        };
        let entries = vec![
            (make_entry(1, "ウマA"), win_only(0.3)),
            (make_entry(2, "ウマB"), win_only(0.1)),
        ];
        let probs = estimate_probabilities(&entries);
        for p in &probs {
            assert!(
                p.win_prob <= p.place_prob && p.place_prob <= p.show_prob,
                "non-monotonic: {p:?}"
            );
            assert!((0.0..=1.0).contains(&p.place_prob));
            assert!((0.0..=1.0).contains(&p.show_prob));
        }
        // 2 頭立てでは place/show 列が合計 0 → 均等フォールバックで min(2/2,1)=min(3/2,1)=1.0。
        for p in &probs {
            assert!((p.place_prob - 1.0).abs() < 1e-10);
            assert!((p.show_prob - 1.0).abs() < 1e-10);
        }
        // win 列は非ゼロなので通常正規化（合計 1.0）。
        let win_total: f64 = probs.iter().map(|p| p.win_prob).sum();
        assert!((win_total - 1.0).abs() < 1e-10);
    }

    /// 騎手なし馬が欠落項で不当に減点されないこと（重み付き平均）。レートが全 factor で等しいなら
    /// 騎手の有無でスコアは変わらず、騎手項は「平均からの差」としてのみ効く。
    #[test]
    fn jockey_none_not_penalized() {
        let base = RateTriple {
            win: 0.2,
            place: 0.4,
            show: 0.6,
        };
        // 騎手レートが他 factor と等しい → 平均不変 → スコアは騎手なしと一致（減点なし）。
        let with_equal_jockey = HorseFactors {
            course_gate: base,
            horse_surface: base,
            horse_distance: base,
            jockey_surface: Some(base),
        };
        let without_jockey = HorseFactors {
            course_gate: base,
            horse_surface: base,
            horse_distance: base,
            jockey_surface: None,
        };
        let s_with = raw_score(&with_equal_jockey, |r| r.win);
        let s_without = raw_score(&without_jockey, |r| r.win);
        assert!(
            (s_with - s_without).abs() < 1e-10,
            "騎手なしが減点されている: with={s_with}, without={s_without}"
        );
        assert!((s_without - 0.2).abs() < 1e-10);

        // 強い騎手（高レート）は加点、弱い騎手（低レート）は減点として正しく効く。
        let strong = HorseFactors {
            jockey_surface: Some(RateTriple {
                win: 0.5,
                place: 0.5,
                show: 0.5,
            }),
            ..with_equal_jockey.clone()
        };
        let weak = HorseFactors {
            jockey_surface: Some(RateTriple::default()),
            ..with_equal_jockey
        };
        assert!(raw_score(&strong, |r| r.win) > s_without);
        assert!(raw_score(&weak, |r| r.win) < s_without);
    }
}
