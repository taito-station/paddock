//! `prediction` モジュールの単体テスト（確率推定・採点・フォーム・パース・縮約・recency・ブレンド）。
//! 挙動が複数サブモジュールに跨るため、テストは概念別に集約してここで保持する。

use std::collections::HashMap;

use chrono::NaiveDate;

use super::parse::parse_margin_lengths;
use super::scoring::{margin_form, raw_score, shrink_rate, time_form};
use super::weights::{
    MARGIN_CAP_LENGTHS, PRIOR_RATE, TIME_DEV_CAP, WEIGHT_CARRIED_CAP_KG, WEIGHT_CHANGE_CAP,
};
use super::*;
use crate::horse_result::{
    FinishingPosition, GateNum, HorseName, HorseNum, HorseResult, ResultStatus, TimeSeconds,
};
use crate::race_card::HorseEntry;

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "expected {b}, got {a}");
}

fn prev_result(
    weight_change: Option<i32>,
    popularity: Option<u32>,
    finish: Option<u32>,
) -> HorseResult {
    HorseResult {
        finishing_position: finish.map(|p| FinishingPosition::try_from(p).unwrap()),
        status: ResultStatus::Finished,
        gate_num: GateNum::try_from(1u32).unwrap(),
        horse_num: HorseNum::try_from(1u32).unwrap(),
        horse_name: HorseName::try_from("ウマ").unwrap(),
        horse_id: None,
        jockey: None,
        trainer: None,
        time_seconds: None,
        margin: None,
        odds: None,
        horse_weight: None,
        weight_change,
        weight_carried: None,
        popularity,
    }
}

fn make_entry(horse_num: u32, horse_name: &str) -> HorseEntry {
    HorseEntry {
        gate_num: crate::horse_result::GateNum::try_from(1u32).unwrap(),
        horse_num: HorseNum::try_from(horse_num).unwrap(),
        horse_name: HorseName::try_from(horse_name).unwrap(),
        jockey: None,
        trainer: None,
        weight_carried: None,
    }
}

/// テスト用: レートを `FactorStat`（出走数 10）に包む。ベイズ縮約 off（`EstimationConfig::default`）
/// の挙動不変テストでは `starts` の値は結果に影響しない。縮約挙動のテストでは starts を明示する。
fn fs(rate: RateTriple) -> FactorStat {
    FactorStat { rate, starts: 10 }
}

fn zero_factors() -> HorseFactors {
    HorseFactors {
        course_gate: Some(fs(RateTriple::default())),
        horse_surface: Some(fs(RateTriple::default())),
        horse_distance: Some(fs(RateTriple::default())),
        jockey_surface: None,
        horse_track_condition: None,
        trainer_surface: None,
        recent_form: None,
        weight_carried: None,
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
fn all_factors_none_scores_zero_and_falls_back_uniform() {
    // 全 factor 欠落（どの統計も実績なし）の馬は weight==0 → raw_score=0.0（NaN でない）。
    let none_factors = HorseFactors {
        course_gate: None,
        horse_surface: None,
        horse_distance: None,
        jockey_surface: None,
        horse_track_condition: None,
        trainer_surface: None,
        recent_form: None,
        weight_carried: None,
    };
    // assert_eq! は NaN（0/0 のゼロ除算）でも 0.0 と不一致で失敗するため NaN 回避も兼ねる。
    let s = raw_score(&none_factors, |r| r.win, &EstimationConfig::default());
    assert_eq!(s, 0.0, "all-None must score finite 0.0, got {s}");

    // estimate_probabilities は全スコア 0 → 均等フォールバック（2 頭なら win=0.5）。
    let entries = vec![
        (make_entry(1, "ウマA"), none_factors.clone()),
        (make_entry(2, "ウマB"), none_factors),
    ];
    let probs = estimate_probabilities(&entries);
    for p in &probs {
        assert!((p.win_prob - 0.5).abs() < 1e-10);
        assert!(p.win_prob <= p.place_prob && p.place_prob <= p.show_prob);
    }
}

/// #81 の核心: 「実績なし」を `None`（母数除外）にすると、0 埋め（`Some(0-rate)`＝全敗扱い）
/// より不当に減点されないこと。他 factor が等しく正のレートなら、除外馬はその平均を維持する。
#[test]
fn missing_record_excluded_is_not_penalized_like_zero_fill() {
    let base = RateTriple {
        win: 0.2,
        place: 0.4,
        show: 0.6,
    };
    // horse_surface の実績なし → None（母数除外）。残り course_gate/distance の平均 0.2 を維持。
    let excluded = HorseFactors {
        course_gate: Some(fs(base)),
        horse_surface: None,
        horse_distance: Some(fs(base)),
        jockey_surface: None,
        horse_track_condition: None,
        trainer_surface: None,
        recent_form: None,
        weight_carried: None,
    };
    // 旧挙動相当: horse_surface=Some(0-rate) は母数に残り平均を押し下げる（＝減点）。
    let zero_filled = HorseFactors {
        horse_surface: Some(fs(RateTriple::default())),
        ..excluded.clone()
    };
    let s_excluded = raw_score(&excluded, |r| r.win, &EstimationConfig::default());
    let s_zero = raw_score(&zero_filled, |r| r.win, &EstimationConfig::default());
    assert!((s_excluded - 0.2).abs() < 1e-10, "excluded={s_excluded}");
    assert!(
        s_excluded > s_zero,
        "実績なし除外は 0 埋めより高評価であるべき: excluded={s_excluded}, zero={s_zero}"
    );
}

#[test]
fn win_sums_to_one_and_values_monotone_small_field() {
    let entries = vec![
        (
            make_entry(1, "ウマA"),
            HorseFactors {
                course_gate: Some(fs(RateTriple {
                    win: 0.2,
                    place: 0.4,
                    show: 0.6,
                })),
                horse_surface: Some(fs(RateTriple {
                    win: 0.1,
                    place: 0.2,
                    show: 0.3,
                })),
                horse_distance: Some(fs(RateTriple {
                    win: 0.15,
                    place: 0.3,
                    show: 0.45,
                })),
                jockey_surface: None,
                horse_track_condition: None,
                trainer_surface: None,
                recent_form: None,
                weight_carried: None,
            },
        ),
        (
            make_entry(2, "ウマB"),
            HorseFactors {
                course_gate: Some(fs(RateTriple {
                    win: 0.1,
                    place: 0.2,
                    show: 0.3,
                })),
                horse_surface: Some(fs(RateTriple {
                    win: 0.05,
                    place: 0.1,
                    show: 0.15,
                })),
                horse_distance: Some(fs(RateTriple {
                    win: 0.08,
                    place: 0.16,
                    show: 0.24,
                })),
                jockey_surface: None,
                horse_track_condition: None,
                trainer_surface: None,
                recent_form: None,
                weight_carried: None,
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
        course_gate: Some(fs(triple)),
        horse_surface: Some(fs(triple)),
        horse_distance: Some(fs(triple)),
        jockey_surface: None,
        horse_track_condition: None,
        trainer_surface: None,
        recent_form: None,
        weight_carried: None,
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
        course_gate: Some(fs(RateTriple {
            win: 0.9,
            place: 0.1,
            show: 0.1,
        })),
        horse_surface: Some(fs(RateTriple::default())),
        horse_distance: Some(fs(RateTriple::default())),
        jockey_surface: None,
        horse_track_condition: None,
        trainer_surface: None,
        recent_form: None,
        weight_carried: None,
    };
    let b = HorseFactors {
        course_gate: Some(fs(RateTriple {
            win: 0.1,
            place: 0.9,
            show: 0.9,
        })),
        horse_surface: Some(fs(RateTriple::default())),
        horse_distance: Some(fs(RateTriple::default())),
        jockey_surface: None,
        horse_track_condition: None,
        trainer_surface: None,
        recent_form: None,
        weight_carried: None,
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
        course_gate: Some(fs(RateTriple {
            win: w,
            place: 0.0,
            show: 0.0,
        })),
        horse_surface: Some(fs(RateTriple::default())),
        horse_distance: Some(fs(RateTriple::default())),
        jockey_surface: None,
        horse_track_condition: None,
        trainer_surface: None,
        recent_form: None,
        weight_carried: None,
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
        course_gate: Some(fs(base)),
        horse_surface: Some(fs(base)),
        horse_distance: Some(fs(base)),
        jockey_surface: Some(fs(base)),
        horse_track_condition: None,
        trainer_surface: None,
        recent_form: None,
        weight_carried: None,
    };
    let without_jockey = HorseFactors {
        course_gate: Some(fs(base)),
        horse_surface: Some(fs(base)),
        horse_distance: Some(fs(base)),
        jockey_surface: None,
        horse_track_condition: None,
        trainer_surface: None,
        recent_form: None,
        weight_carried: None,
    };
    let s_with = raw_score(&with_equal_jockey, |r| r.win, &EstimationConfig::default());
    let s_without = raw_score(&without_jockey, |r| r.win, &EstimationConfig::default());
    assert!(
        (s_with - s_without).abs() < 1e-10,
        "騎手なしが減点されている: with={s_with}, without={s_without}"
    );
    assert!((s_without - 0.2).abs() < 1e-10);

    // 強い騎手（高レート）は加点、弱い騎手（低レート）は減点として正しく効く。
    let strong = HorseFactors {
        jockey_surface: Some(fs(RateTriple {
            win: 0.5,
            place: 0.5,
            show: 0.5,
        })),
        ..with_equal_jockey.clone()
    };
    let weak = HorseFactors {
        jockey_surface: Some(fs(RateTriple::default())),
        ..with_equal_jockey
    };
    assert!(raw_score(&strong, |r| r.win, &EstimationConfig::default()) > s_without);
    assert!(raw_score(&weak, |r| r.win, &EstimationConfig::default()) < s_without);
}

/// 馬場状態項（#73）が欠落項で不当に減点されないこと（重み付き平均、ADR 0007 の流儀）。
/// レートが全 factor で等しいなら馬場項の有無でスコアは変わらず、「平均からの差」としてのみ効く。
#[test]
fn track_condition_none_not_penalized() {
    let base = RateTriple {
        win: 0.2,
        place: 0.4,
        show: 0.6,
    };
    let with_equal_tc = HorseFactors {
        course_gate: Some(fs(base)),
        horse_surface: Some(fs(base)),
        horse_distance: Some(fs(base)),
        jockey_surface: None,
        horse_track_condition: Some(fs(base)),
        trainer_surface: None,
        recent_form: None,
        weight_carried: None,
    };
    let without_tc = HorseFactors {
        horse_track_condition: None,
        ..with_equal_tc.clone()
    };
    let s_with = raw_score(&with_equal_tc, |r| r.win, &EstimationConfig::default());
    let s_without = raw_score(&without_tc, |r| r.win, &EstimationConfig::default());
    assert!(
        (s_with - s_without).abs() < 1e-10,
        "馬場実績なしが減点されている: with={s_with}, without={s_without}"
    );
    assert!((s_without - 0.2).abs() < 1e-10);

    // 道悪巧者（高レート）は加点、苦手（低レート）は減点として正しく効く。
    let strong = HorseFactors {
        horse_track_condition: Some(fs(RateTriple {
            win: 0.5,
            place: 0.5,
            show: 0.5,
        })),
        ..with_equal_tc.clone()
    };
    let weak = HorseFactors {
        horse_track_condition: Some(fs(RateTriple::default())),
        ..with_equal_tc
    };
    assert!(raw_score(&strong, |r| r.win, &EstimationConfig::default()) > s_without);
    assert!(raw_score(&weak, |r| r.win, &EstimationConfig::default()) < s_without);
}

/// 馬場状態項を含む場合でも win ≤ place ≤ show の単調性が維持されること（#73）。
#[test]
fn track_condition_keeps_monotonicity_in_estimate() {
    let entries = vec![
        (
            make_entry(1, "ウマA"),
            HorseFactors {
                course_gate: Some(fs(RateTriple {
                    win: 0.3,
                    place: 0.5,
                    show: 0.7,
                })),
                horse_surface: Some(fs(RateTriple {
                    win: 0.2,
                    place: 0.4,
                    show: 0.6,
                })),
                horse_distance: Some(fs(RateTriple {
                    win: 0.1,
                    place: 0.3,
                    show: 0.5,
                })),
                jockey_surface: None,
                // win 偏重の不自然な馬場レートでも単調化が是正する。
                horse_track_condition: Some(fs(RateTriple {
                    win: 0.9,
                    place: 0.1,
                    show: 0.1,
                })),
                trainer_surface: None,
                recent_form: None,
                weight_carried: None,
            },
        ),
        (
            make_entry(2, "ウマB"),
            HorseFactors {
                course_gate: Some(fs(RateTriple {
                    win: 0.1,
                    place: 0.2,
                    show: 0.3,
                })),
                horse_surface: Some(fs(RateTriple {
                    win: 0.1,
                    place: 0.2,
                    show: 0.3,
                })),
                horse_distance: Some(fs(RateTriple {
                    win: 0.1,
                    place: 0.2,
                    show: 0.3,
                })),
                jockey_surface: None,
                horse_track_condition: None,
                trainer_surface: None,
                recent_form: None,
                weight_carried: None,
            },
        ),
    ];
    let probs = estimate_probabilities(&entries);
    for p in &probs {
        assert!(
            p.win_prob <= p.place_prob + 1e-10 && p.place_prob <= p.show_prob + 1e-10,
            "単調性違反: {} win={} place={} show={}",
            p.horse_name.value(),
            p.win_prob,
            p.place_prob,
            p.show_prob
        );
    }
}

/// 調教師項（#74）が欠落項で不当に減点されないこと（重み付き平均、ADR 0007 の流儀）。
/// レートが全 factor で等しいなら調教師項の有無でスコアは変わらず、「平均からの差」としてのみ効く。
#[test]
fn trainer_absent_not_penalized() {
    let base = RateTriple {
        win: 0.2,
        place: 0.4,
        show: 0.6,
    };
    let with_equal_trainer = HorseFactors {
        course_gate: Some(fs(base)),
        horse_surface: Some(fs(base)),
        horse_distance: Some(fs(base)),
        jockey_surface: None,
        trainer_surface: Some(fs(base)),
        horse_track_condition: None,
        recent_form: None,
        weight_carried: None,
    };
    let without_trainer = HorseFactors {
        trainer_surface: None,
        ..with_equal_trainer.clone()
    };
    let s_with = raw_score(&with_equal_trainer, |r| r.win, &EstimationConfig::default());
    let s_without = raw_score(&without_trainer, |r| r.win, &EstimationConfig::default());
    assert!(
        (s_with - s_without).abs() < 1e-10,
        "調教師実績なしが減点されている: with={s_with}, without={s_without}"
    );
    assert!((s_without - 0.2).abs() < 1e-10);

    // 名伯楽（高レート）は加点、苦手（低レート）は減点として正しく効く。
    let strong = HorseFactors {
        trainer_surface: Some(fs(RateTriple {
            win: 0.5,
            place: 0.5,
            show: 0.5,
        })),
        ..with_equal_trainer.clone()
    };
    let weak = HorseFactors {
        trainer_surface: Some(fs(RateTriple::default())),
        ..with_equal_trainer
    };
    assert!(raw_score(&strong, |r| r.win, &EstimationConfig::default()) > s_without);
    assert!(raw_score(&weak, |r| r.win, &EstimationConfig::default()) < s_without);
}

// ---- ベイズ縮約（#75） ----

fn shrink_cfg(m: f64) -> EstimationConfig {
    EstimationConfig {
        shrinkage: Some(ShrinkageConfig { pseudo_count: m }),
        recency: None,
    }
}

#[test]
fn shrink_rate_endpoints_and_monotonic() {
    let prior = 0.1;
    let m = 10.0;
    // k=0（実績ゼロ相当）は完全に prior。
    assert!((shrink_rate(0.9, 0, prior, m) - prior).abs() < 1e-12);
    // k≫m は ≈ 生レート（縮約がほぼ効かない）。
    assert!((shrink_rate(0.9, 100_000, prior, m) - 0.9).abs() < 1e-3);
    // k=m なら生レートと prior のちょうど中点。
    assert!((shrink_rate(0.9, 10, prior, m) - (0.9 + prior) / 2.0).abs() < 1e-12);
    // starts が増えるほど生レートへ単調に近づく（prior より高いレートで単調増加）。
    let s1 = shrink_rate(0.9, 1, prior, m);
    let s5 = shrink_rate(0.9, 5, prior, m);
    let s20 = shrink_rate(0.9, 20, prior, m);
    assert!(prior < s1 && s1 < s5 && s5 < s20 && s20 < 0.9);
}

/// 少データ（starts 小）の高レート factor は縮約で prior 側へ強く引かれ、
/// 同じレートでも大データ（starts 大）より低いスコアになる。
#[test]
fn shrinkage_pulls_low_sample_toward_prior() {
    let high_rate = RateTriple {
        win: 0.8,
        place: 0.8,
        show: 0.8,
    };
    let few = HorseFactors {
        course_gate: Some(FactorStat {
            rate: high_rate,
            starts: 1,
        }),
        horse_surface: None,
        horse_distance: None,
        jockey_surface: None,
        trainer_surface: None,
        horse_track_condition: None,
        recent_form: None,
        weight_carried: None,
    };
    let many = HorseFactors {
        course_gate: Some(FactorStat {
            rate: high_rate,
            starts: 200,
        }),
        ..few.clone()
    };
    let cfg = shrink_cfg(10.0);
    let s_few = raw_score(&few, |r| r.win, &cfg);
    let s_many = raw_score(&many, |r| r.win, &cfg);
    // prior(=1/14≈0.071) < 少データ < 多データ < 生レート(0.8)。
    assert!(PRIOR_RATE.win < s_few && s_few < s_many && s_many < 0.8);
    // 縮約 off では starts に依らず生レートのまま（挙動不変の確認）。
    let off = EstimationConfig::default();
    assert!((raw_score(&few, |r| r.win, &off) - 0.8).abs() < 1e-12);
    assert!((raw_score(&many, |r| r.win, &off) - 0.8).abs() < 1e-12);
}

/// 少データ馬が他の有力馬と同居しても、縮約により win_prob が 0 へ振り切れず
/// 正値を保つ（ADR 0002 の `win_prob=0` 緩和）。
#[test]
fn shrinkage_keeps_low_sample_horse_above_zero() {
    // 1 頭は実績豊富で高レート、もう 1 頭は少データ（starts=1）で低レート。
    let strong = HorseFactors {
        course_gate: Some(FactorStat {
            rate: RateTriple {
                win: 0.6,
                place: 0.6,
                show: 0.6,
            },
            starts: 100,
        }),
        horse_surface: None,
        horse_distance: None,
        jockey_surface: None,
        trainer_surface: None,
        horse_track_condition: None,
        recent_form: None,
        weight_carried: None,
    };
    let sparse = HorseFactors {
        course_gate: Some(FactorStat {
            rate: RateTriple::default(),
            starts: 1,
        }),
        ..strong.clone()
    };
    let entries = vec![
        (make_entry(1, "ウマ強"), strong),
        (make_entry(2, "ウマ薄"), sparse),
    ];
    let probs = estimate_probabilities_with_config(&entries, &shrink_cfg(10.0));
    let sparse_win = probs[1].win_prob;
    // 縮約により prior 方向へ持ち上がり、0 より大きい有限値になる。
    assert!(
        sparse_win > 0.0 && sparse_win.is_finite(),
        "sparse_win={sparse_win}"
    );
    // ただし強い馬よりは低い（順位は保つ）。
    assert!(probs[0].win_prob > sparse_win);
}

/// 本番 predict（`predict_race`）が使う `production()` の設定を固定する回帰ガード。
/// 縮約 m を取り違えたり recency を誤って有効化すると CI で検知する（#75/ADR 0016）。
#[test]
fn production_config_is_shrinkage_m10_and_recency_off() {
    let c = EstimationConfig::production();
    assert_eq!(
        c.shrinkage.expect("production は縮約 on").pseudo_count,
        RECOMMENDED_SHRINKAGE_M
    );
    assert!((RECOMMENDED_SHRINKAGE_M - 10.0).abs() < 1e-12);
    assert!(
        c.recency.is_none(),
        "recency は backtest 評価で無効採用（ADR 0016）"
    );
}

// ---- リーセンシー重み付け（#75 Phase B） ----

fn dc(date: NaiveDate, starts: u32, wins: u32) -> DatedCounts {
    DatedCounts {
        date,
        starts,
        wins,
        places: wins,
        shows: wins,
    }
}

#[test]
fn recency_empty_or_all_future_is_none() {
    let as_of = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    assert!(apply_recency_weight(&[], as_of, 30.0).is_none());
    // as_of 当日・以降のみ → 全て無視され None（リーク防止）。
    let future = [
        dc(as_of, 1, 1),
        dc(NaiveDate::from_ymd_opt(2026, 6, 2).unwrap(), 1, 1),
    ];
    assert!(apply_recency_weight(&future, as_of, 30.0).is_none());
}

#[test]
fn recency_weights_recent_runs_higher() {
    let as_of = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    // 直近 1 走で勝ち、半減期 1 つ前（30 日前）に負け。重みは直近が 2 倍なので
    // 重み付き勝率は単純平均 0.5 より高くなる。
    let runs = [
        dc(NaiveDate::from_ymd_opt(2026, 5, 2).unwrap(), 1, 0), // 30 日前: 着外
        dc(NaiveDate::from_ymd_opt(2026, 5, 31).unwrap(), 1, 1), // 1 日前: 勝ち
    ];
    let fs = apply_recency_weight(&runs, as_of, 30.0).expect("some");
    assert!(
        fs.rate.win > 0.5,
        "直近の勝ちが重く効くべき: {}",
        fs.rate.win
    );
    // 総出走数は時間重みを掛けない素の値。
    assert_eq!(fs.starts, 2);
}

#[test]
fn recency_half_life_halves_weight() {
    let as_of = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    // half_life=30 日。直近(1 日前)勝ち1走 + 30 日前負け1走の重み比 ≈ 2:1。
    // 期待勝率 = w_recent / (w_recent + w_old)。
    let recent = (as_of - NaiveDate::from_ymd_opt(2026, 5, 31).unwrap()).num_days() as f64;
    let old = (as_of - NaiveDate::from_ymd_opt(2026, 5, 2).unwrap()).num_days() as f64;
    let w_recent = 0.5_f64.powf(recent / 30.0);
    let w_old = 0.5_f64.powf(old / 30.0);
    let expected = w_recent / (w_recent + w_old);
    let runs = [
        dc(NaiveDate::from_ymd_opt(2026, 5, 2).unwrap(), 1, 0),
        dc(NaiveDate::from_ymd_opt(2026, 5, 31).unwrap(), 1, 1),
    ];
    let fs = apply_recency_weight(&runs, as_of, 30.0).expect("some");
    assert!(
        (fs.rate.win - expected).abs() < 1e-12,
        "win={} expected={expected}",
        fs.rate.win
    );
}

fn prob(num: u32, win: f64, place: f64, show: f64) -> HorseProbability {
    HorseProbability {
        horse_num: HorseNum::try_from(num).unwrap(),
        horse_name: HorseName::try_from(format!("ウマ{num}")).unwrap(),
        win_prob: win,
        place_prob: place,
        show_prob: show,
    }
}

fn odds_map(pairs: &[(u32, f64)]) -> HashMap<HorseNum, f64> {
    pairs
        .iter()
        .map(|&(n, o)| (HorseNum::try_from(n).unwrap(), o))
        .collect()
}

#[test]
fn blend_alpha_one_is_noop() {
    let probs = vec![prob(1, 0.6, 0.7, 0.8), prob(2, 0.4, 0.5, 0.6)];
    let out = blend_with_market_win(&probs, &odds_map(&[(1, 2.0), (2, 2.0)]), 1.0);
    for (a, b) in probs.iter().zip(&out) {
        approx(a.win_prob, b.win_prob);
        approx(a.place_prob, b.place_prob);
        approx(a.show_prob, b.show_prob);
    }
}

#[test]
fn blend_empty_market_is_noop() {
    let probs = vec![prob(1, 0.6, 0.7, 0.8), prob(2, 0.4, 0.5, 0.6)];
    let out = blend_with_market_win(&probs, &HashMap::new(), 0.5);
    assert_eq!(out.len(), 2);
    approx(out[0].win_prob, 0.6);
    approx(out[1].win_prob, 0.4);
}

#[test]
fn blend_removes_overround_and_mixes() {
    // モデル win = [0.5, 0.5]、オッズ [1.5, 3.0]。
    // implied = [0.6667, 0.3333], overround=1.0 → market_prob = [0.6667, 0.3333]
    // （このオッズは控除率0なので偶然 overround=1.0）。α=0.5 →
    // blended = [0.5*0.5+0.5*0.6667, 0.5*0.5+0.5*0.3333] = [0.5833, 0.4167]、合計1.0。
    let probs = vec![prob(1, 0.5, 0.6, 0.7), prob(2, 0.5, 0.6, 0.7)];
    let out = blend_with_market_win(&probs, &odds_map(&[(1, 1.5), (2, 3.0)]), 0.5);
    let m1 = (1.0 / 1.5) / (1.0 / 1.5 + 1.0 / 3.0);
    approx(out[0].win_prob, 0.5 * 0.5 + 0.5 * m1);
    approx(out[1].win_prob, 1.0 - out[0].win_prob);
    let total: f64 = out.iter().map(|p| p.win_prob).sum();
    approx(total, 1.0);
}

#[test]
fn blend_normalizes_when_overround_above_one() {
    // 控除率あり: オッズ [1.5, 1.5] → implied=[0.667,0.667] overround=1.333 → market=[0.5,0.5]。
    let probs = vec![prob(1, 0.7, 0.8, 0.9), prob(2, 0.3, 0.4, 0.5)];
    let out = blend_with_market_win(&probs, &odds_map(&[(1, 1.5), (2, 1.5)]), 0.5);
    // market = [0.5,0.5]、blended=[0.6,0.4]、合計1.0。
    approx(out[0].win_prob, 0.6);
    approx(out[1].win_prob, 0.4);
    let total: f64 = out.iter().map(|p| p.win_prob).sum();
    approx(total, 1.0);
}

#[test]
fn blend_keeps_monotonicity_and_unit_range() {
    // 市場が favorite の win を model.place 超へ押し上げても win ≤ place ≤ show を保つ。
    let probs = vec![prob(1, 0.4, 0.45, 0.5), prob(2, 0.6, 0.62, 0.7)];
    let out = blend_with_market_win(&probs, &odds_map(&[(1, 1.2), (2, 6.0)]), 0.2);
    for p in &out {
        assert!(
            p.win_prob <= p.place_prob && p.place_prob <= p.show_prob,
            "{p:?}"
        );
        assert!((0.0..=1.0).contains(&p.win_prob));
        assert!((0.0..=1.0).contains(&p.show_prob));
    }
}

#[test]
fn blend_non_finite_alpha_is_noop() {
    // 非有限 α（NaN）は防御的に no-op（CLI で弾く前提だがドメイン単体でも保証）。
    let probs = vec![prob(1, 0.6, 0.7, 0.8), prob(2, 0.4, 0.5, 0.6)];
    let out = blend_with_market_win(&probs, &odds_map(&[(1, 2.0), (2, 2.0)]), f64::NAN);
    approx(out[0].win_prob, 0.6);
    approx(out[1].win_prob, 0.4);
}

#[test]
fn blend_noop_when_all_odds_nonpositive() {
    // 全オッズが 0/負（型検証を経ない生 f64 経路の異常値）→ implied 空 → overround 0 → no-op。
    let probs = vec![prob(1, 0.6, 0.7, 0.8), prob(2, 0.4, 0.5, 0.6)];
    let out = blend_with_market_win(&probs, &odds_map(&[(1, 0.0), (2, -1.0)]), 0.5);
    approx(out[0].win_prob, 0.6);
    approx(out[1].win_prob, 0.4);
}

#[test]
fn blend_partial_coverage_keeps_model_for_missing_and_renormalizes() {
    // 馬2 はオッズ無し → モデル値を保ちつつ全体は合計1.0へ再正規化。
    let probs = vec![prob(1, 0.5, 0.6, 0.7), prob(2, 0.5, 0.6, 0.7)];
    let out = blend_with_market_win(&probs, &odds_map(&[(1, 1.1)]), 0.5);
    let total: f64 = out.iter().map(|p| p.win_prob).sum();
    approx(total, 1.0);
    // 馬1 は超 favorite オッズなので blend で win が上がる。
    assert!(out[0].win_prob > out[1].win_prob);
}

#[test]
fn recent_form_none_when_no_signals() {
    // 体重変化・人気・着順すべて欠損、かつ前走間隔も非正（同日）→ signal 無し → None。
    let prev = prev_result(None, None, None);
    assert!(recent_form_score(&prev, ymd(2026, 5, 1), ymd(2026, 5, 1), None).is_none());
}

#[test]
fn recent_form_weight_change_smaller_is_better() {
    let d = ymd(2026, 5, 1);
    let pd = ymd(2026, 4, 1); // 30 日前（最適帯 1.0）
    // 体重変化のみで比較するため人気・着順は欠損。
    let stable = recent_form_score(&prev_result(Some(2), None, None), pd, d, None).unwrap();
    let swingy = recent_form_score(&prev_result(Some(18), None, None), pd, d, None).unwrap();
    assert!(stable > swingy, "stable={stable}, swingy={swingy}");
    // CAP(20kg) 超は体重 signal が 0。間隔 signal(1.0) との平均なので 0.5。
    let huge = recent_form_score(&prev_result(Some(40), None, None), pd, d, None).unwrap();
    assert!((huge - 0.5).abs() < 1e-9, "huge={huge}");
}

#[test]
fn recent_form_popularity_gap() {
    let d = ymd(2026, 5, 1);
    let pd = ymd(2026, 4, 1);
    // 5 番人気で 2 着（人気以上に好走）→ 加点。1 番人気で 8 着（凡走）→ 減点。
    let over = recent_form_score(&prev_result(None, Some(5), Some(2)), pd, d, None).unwrap();
    let under = recent_form_score(&prev_result(None, Some(1), Some(8)), pd, d, None).unwrap();
    assert!(over > under, "over={over}, under={under}");
}

#[test]
fn recent_form_interval_band() {
    // 最適帯(30日)=1.0、連闘(3日)=0.3、長休(200日)=0.5。間隔のみ（他欠損）。
    let base = ymd(2026, 5, 1);
    let optimal = recent_form_score(&prev_result(None, None, None), ymd(2026, 4, 1), base, None);
    let rento = recent_form_score(&prev_result(None, None, None), ymd(2026, 4, 28), base, None);
    let layoff = recent_form_score(
        &prev_result(None, None, None),
        ymd(2025, 10, 13),
        base,
        None,
    );
    assert!((optimal.unwrap() - 1.0).abs() < 1e-9);
    assert!((rento.unwrap() - 0.3).abs() < 1e-9);
    assert!((layoff.unwrap() - 0.5).abs() < 1e-9);
}

#[test]
fn recent_form_drops_popularity_signal_when_no_finish() {
    // 着順なし（中止・失格等で finishing_position = None）の前走は、人気が取れていても
    // 人気乖離 signal を落とし、体重変化(0.9: Δ=2)と間隔(1.0: 30日)のみで算出 → 平均 0.95。
    let d = ymd(2026, 5, 1);
    let pd = ymd(2026, 4, 1);
    let f = recent_form_score(&prev_result(Some(2), Some(3), None), pd, d, None).unwrap();
    let weight_sig = 1.0 - 2.0 / WEIGHT_CHANGE_CAP; // 0.9
    assert!((f - (weight_sig + 1.0) / 2.0).abs() < 1e-9, "form={f}");
}

#[test]
fn recent_form_in_unit_range() {
    // 全 signal が揃ったケースでも [0,1]。
    let d = ymd(2026, 5, 1);
    let pd = ymd(2026, 4, 10);
    let f = recent_form_score(&prev_result(Some(-4), Some(3), Some(1)), pd, d, None).unwrap();
    assert!((0.0..=1.0).contains(&f), "form={f}");
}

#[test]
fn parse_margin_keywords() {
    approx(parse_margin_lengths("ハナ").unwrap(), 0.05);
    approx(parse_margin_lengths("アタマ").unwrap(), 0.10);
    approx(parse_margin_lengths("クビ").unwrap(), 0.25);
    approx(parse_margin_lengths("同着").unwrap(), 0.0);
    approx(parse_margin_lengths("大差").unwrap(), MARGIN_CAP_LENGTHS);
}

#[test]
fn parse_margin_fractions_and_decimals() {
    approx(parse_margin_lengths("1/2").unwrap(), 0.5);
    approx(parse_margin_lengths("3/4").unwrap(), 0.75);
    approx(parse_margin_lengths("1.1/4").unwrap(), 1.25); // 整数1 + 分数1/4
    approx(parse_margin_lengths("2.1/2").unwrap(), 2.5);
    approx(parse_margin_lengths("0.6").unwrap(), 0.6); // netkeiba 形式の小数
    approx(parse_margin_lengths("2").unwrap(), 2.0);
    approx(parse_margin_lengths(" 1.0 ").unwrap(), 1.0); // 前後空白を許容
}

#[test]
fn parse_margin_invalid_is_none() {
    // 空・記号のみ・分母0・非数値・負値に加え、整数+分数の分母0（1.1/0）・分子非数値（abc/2）・
    // 整数部欠落の先頭ドット（.5/2）も None に倒す。
    for s in [
        "", "   ", "-", "1/0", "abc", "-1.0", "1.1/0", "abc/2", ".5/2", "-1/2",
    ] {
        assert!(parse_margin_lengths(s).is_none(), "expected None for {s:?}");
    }
}

#[test]
fn margin_form_win_rewards_dominance() {
    // 圧勝(大差)=1.0、僅差勝ち(0.05馬身)≈0.5 をわずかに上回る。
    let dominant = margin_form(1, MARGIN_CAP_LENGTHS);
    let narrow = margin_form(1, 0.05);
    approx(dominant, 1.0);
    assert!(narrow > 0.5 && narrow < 0.55, "narrow={narrow}");
    assert!(dominant > narrow);
}

#[test]
fn margin_form_loss_penalizes_blowout() {
    // 大敗(大差)=0.0、接戦負け(0.05馬身)≈0.5 をわずかに下回る。
    let blown = margin_form(5, MARGIN_CAP_LENGTHS);
    let close = margin_form(2, 0.05);
    approx(blown, 0.0);
    assert!(close < 0.5 && close > 0.45, "close={close}");
    assert!(blown < close);
}

#[test]
fn recent_form_includes_margin_signal() {
    // 着差以外を欠損させ間隔(30日=1.0)＋着差のみで評価。圧勝(大差勝ち)が大敗を上回る。
    let d = ymd(2026, 5, 1);
    let pd = ymd(2026, 4, 1);
    let mut winner = prev_result(None, None, Some(1));
    winner.margin = Some("大差".to_string());
    let mut loser = prev_result(None, None, Some(10));
    loser.margin = Some("大差".to_string());
    let wf = recent_form_score(&winner, pd, d, None).unwrap();
    let lf = recent_form_score(&loser, pd, d, None).unwrap();
    approx(wf, 1.0); // (間隔1.0 + 着差1.0)/2
    approx(lf, 0.5); // (間隔1.0 + 着差0.0)/2
    assert!(wf > lf);
}

#[test]
fn recent_form_drops_margin_signal_when_unparseable() {
    // 着差が解釈不能なら margin signal を落とし、間隔(30日)のみ → 1.0。
    let d = ymd(2026, 5, 1);
    let pd = ymd(2026, 4, 1);
    let mut prev = prev_result(None, None, Some(3));
    prev.margin = Some("???".to_string());
    approx(recent_form_score(&prev, pd, d, None).unwrap(), 1.0);
}

#[test]
fn time_form_faster_is_higher() {
    // 標準より速い（タイム小）→ >0.5、遅い→ <0.5、同値→0.5。
    let std = 100.0;
    assert!(time_form(98.0, std) > 0.5);
    assert!(time_form(102.0, std) < 0.5);
    approx(time_form(100.0, std), 0.5);
}

#[test]
fn time_form_saturates_at_cap() {
    // CAP(=5%)を超える偏差は 0/1 に飽和。標準非正は中立 0.5（防御）。
    let std = 100.0;
    approx(time_form(std * (1.0 - TIME_DEV_CAP), std), 1.0); // ちょうど CAP 速い → 1.0
    approx(time_form(std * (1.0 - 2.0 * TIME_DEV_CAP), std), 1.0); // CAP 超でも 1.0 にクランプ
    approx(time_form(std * (1.0 + 2.0 * TIME_DEV_CAP), std), 0.0); // CAP 超の遅さ → 0.0
    approx(time_form(95.0, 0.0), 0.5);
}

#[test]
fn recent_form_includes_time_signal() {
    // タイム以外を欠損させ、間隔(30日=1.0)＋タイムのみで評価。標準より速い前走が遅い前走を上回る。
    let d = ymd(2026, 5, 1);
    let pd = ymd(2026, 4, 1);
    let std = Some(100.0);
    let mut fast = prev_result(None, None, None);
    fast.time_seconds = Some(TimeSeconds::try_from(98.0).unwrap());
    let mut slow = prev_result(None, None, None);
    slow.time_seconds = Some(TimeSeconds::try_from(102.0).unwrap());
    let ff = recent_form_score(&fast, pd, d, std).unwrap();
    let sf = recent_form_score(&slow, pd, d, std).unwrap();
    assert!(ff > sf, "fast={ff}, slow={sf}");
    // 標準タイム未整備（None）ならタイム signal は落ち、間隔のみ → 1.0。
    approx(recent_form_score(&fast, pd, d, None).unwrap(), 1.0);
}

#[test]
fn weight_factor_heavier_is_higher() {
    // 採用符号（backtest 検証）: 平均より重い→>0.5、軽い→<0.5、平均同値→0.5。
    let mean = 55.0;
    assert!(weight_factor(57.0, mean) > 0.5);
    assert!(weight_factor(53.0, mean) < 0.5);
    approx(weight_factor(55.0, mean), 0.5);
}

#[test]
fn weight_factor_saturates_and_guards() {
    // CAP(=3kg) でちょうど 1/0 に飽和、CAP 超でもクランプ。field_mean 非正は中立 0.5。
    let mean = 55.0;
    approx(weight_factor(mean + WEIGHT_CARRIED_CAP_KG, mean), 1.0);
    approx(weight_factor(mean - WEIGHT_CARRIED_CAP_KG, mean), 0.0);
    approx(weight_factor(mean + 2.0 * WEIGHT_CARRIED_CAP_KG, mean), 1.0);
    approx(weight_factor(56.0, 0.0), 0.5);
    // 非有限入力（NaN/inf）は中立 0.5（NaN を出力して全馬の確率を汚染しないための防御）。
    approx(weight_factor(56.0, f64::NAN), 0.5);
    approx(weight_factor(f64::NAN, mean), 0.5);
    approx(weight_factor(f64::INFINITY, mean), 0.5);
}

#[test]
fn weight_carried_factor_shifts_score_and_excluded_when_none() {
    // weight_carried 項（スカラー [0,1]）が raw_score に効く（高い factor 値→高スコア）／
    // None で母数から落ちる。値の向き（重い→高い）は weight_factor 側のテストで担保。
    let mut low = zero_factors();
    low.weight_carried = Some(0.2);
    let mut high = zero_factors();
    high.weight_carried = Some(0.8);
    let cfg = EstimationConfig::default();
    let s_low = raw_score(&low, |r| r.win, &cfg);
    let s_high = raw_score(&high, |r| r.win, &cfg);
    assert!(s_high > s_low, "high={s_high}, low={s_low}");
    // None なら項なし（zero_factors と同一スコア）。
    let base = raw_score(&zero_factors(), |r| r.win, &cfg);
    let mut none_w = zero_factors();
    none_w.weight_carried = None;
    approx(raw_score(&none_w, |r| r.win, &cfg), base);
}

#[test]
fn recent_form_keeps_monotonicity_in_estimate() {
    // recent_form を持つ馬・持たない馬が混在しても単調性は保たれる。
    let mut f_with = zero_factors();
    f_with.course_gate = Some(fs(RateTriple {
        win: 0.3,
        place: 0.4,
        show: 0.5,
    }));
    f_with.recent_form = Some(0.9);
    let mut f_without = zero_factors();
    f_without.course_gate = Some(fs(RateTriple {
        win: 0.2,
        place: 0.3,
        show: 0.4,
    }));
    let entries = vec![
        (make_entry(1, "ウマA"), f_with),
        (make_entry(2, "ウマB"), f_without),
    ];
    let probs = estimate_probabilities(&entries);
    for p in &probs {
        assert!(p.win_prob <= p.place_prob && p.place_prob <= p.show_prob);
    }
}
