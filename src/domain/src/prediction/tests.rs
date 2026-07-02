//! `prediction` モジュールの単体テスト（確率推定・採点・フォーム・パース・縮約・recency・ブレンド）。
//! 挙動が複数サブモジュールに跨るため、テストは概念別に集約してここで保持する。

use std::collections::HashMap;

use chrono::NaiveDate;

use super::parse::{parse_corner_positions, parse_margin_lengths};
use super::scoring::{
    FactorImpute, jockey_recent_form_score, leading_position, margin_form, raw_score,
    raw_score_with_impute, shrink_rate, time_form,
};
use super::weights::{
    JOCKEY_RECENT_FORM_WEIGHT, MARGIN_CAP_LENGTHS, PRIOR_RATE, TIME_DEV_CAP, WEIGHT_CARRIED_CAP_KG,
    WEIGHT_CHANGE_CAP,
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
        jockey_recent_form: None,
        running_style: None,
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
        jockey_recent_form: None,
        running_style: None,
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
        jockey_recent_form: None,
        running_style: None,
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
                jockey_recent_form: None,
                running_style: None,
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
                jockey_recent_form: None,
                running_style: None,
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
        jockey_recent_form: None,
        running_style: None,
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

/// place/show 冪変換（#283）: γ>1 で本命の place/show が持ち上がり人気薄が下がる（脱圧縮）。
/// 上限クランプの起きない 8 頭立てで、場内合計 2.0/3.0 は保たれることも確認する。
#[test]
fn place_show_power_decompresses_toward_favorite() {
    // 1 頭だけ place/show レートが高く、残り 7 頭は低い。win は全馬同一にして
    // 単調化（place を win で floor）が脱圧縮の判定に干渉しないようにする。
    let strong = RateTriple {
        win: 0.1,
        place: 0.25,
        show: 0.3,
    };
    let weak = RateTriple {
        win: 0.1,
        place: 0.15,
        show: 0.2,
    };
    let mk = |t: RateTriple| HorseFactors {
        course_gate: Some(fs(t)),
        horse_surface: Some(fs(t)),
        horse_distance: Some(fs(t)),
        jockey_surface: None,
        horse_track_condition: None,
        trainer_surface: None,
        recent_form: None,
        weight_carried: None,
        jockey_recent_form: None,
        running_style: None,
    };
    let entries: Vec<_> = (1..=8)
        .map(|i| {
            let t = if i == 1 { strong } else { weak };
            (make_entry(i, &format!("ウマ{i}")), mk(t))
        })
        .collect();

    let base = estimate_probabilities(&entries);
    let powered = estimate_probabilities_with_config(
        &entries,
        &EstimationConfig {
            place_show_power: Some(2.0),
            ..EstimationConfig::default()
        },
    );

    // 本命（idx 0）の place/show は上がり、人気薄（idx 1）の place/show は下がる。
    assert!(
        powered[0].place_prob > base[0].place_prob,
        "favorite place up: {} -> {}",
        base[0].place_prob,
        powered[0].place_prob
    );
    assert!(
        powered[0].show_prob > base[0].show_prob,
        "favorite show up: {} -> {}",
        base[0].show_prob,
        powered[0].show_prob
    );
    assert!(
        powered[1].place_prob < base[1].place_prob,
        "longshot place down: {} -> {}",
        base[1].place_prob,
        powered[1].place_prob
    );

    // クランプの無い設定なので場内合計は 2.0 / 3.0 を保つ。
    let place_total: f64 = powered.iter().map(|p| p.place_prob).sum();
    let show_total: f64 = powered.iter().map(|p| p.show_prob).sum();
    assert!(
        (place_total - 2.0).abs() < 1e-9,
        "place_total={place_total}"
    );
    assert!((show_total - 3.0).abs() < 1e-9, "show_total={show_total}");
}

/// place/show 冪変換（#283）: γ=None / γ=1.0 は no-op（後方互換）。win も不変。
#[test]
fn place_show_power_none_or_one_is_noop() {
    let triple = RateTriple {
        win: 0.2,
        place: 0.3,
        show: 0.4,
    };
    let mk = || HorseFactors {
        course_gate: Some(fs(triple)),
        horse_surface: Some(fs(RateTriple {
            win: 0.1,
            place: 0.2,
            show: 0.25,
        })),
        horse_distance: Some(fs(triple)),
        jockey_surface: None,
        horse_track_condition: None,
        trainer_surface: None,
        recent_form: None,
        weight_carried: None,
        jockey_recent_form: None,
        running_style: None,
    };
    // スコアに差がつくよう馬ごとに 1 factor だけ差し替える。
    let entries: Vec<_> = (1..=6)
        .map(|i| {
            let mut f = mk();
            f.horse_surface = Some(fs(RateTriple {
                win: 0.1 * i as f64 / 6.0,
                place: 0.2 * i as f64 / 6.0,
                show: 0.25 * i as f64 / 6.0,
            }));
            (make_entry(i, &format!("ウマ{i}")), f)
        })
        .collect();
    let base = estimate_probabilities(&entries);
    for g in [None, Some(1.0)] {
        let out = estimate_probabilities_with_config(
            &entries,
            &EstimationConfig {
                place_show_power: g,
                ..EstimationConfig::default()
            },
        );
        for (a, b) in base.iter().zip(&out) {
            approx(a.win_prob, b.win_prob);
            approx(a.place_prob, b.place_prob);
            approx(a.show_prob, b.show_prob);
        }
    }
}

/// place/show 冪変換（#283）: γ<1（逆方向＝圧縮）は本命の place/show を下げ人気薄を上げる。
/// help で「γ<1 は逆方向」と宣伝しているため逆向きの挙動も固定する。
#[test]
fn place_show_power_below_one_compresses() {
    let strong = RateTriple {
        win: 0.1,
        place: 0.25,
        show: 0.3,
    };
    let weak = RateTriple {
        win: 0.1,
        place: 0.15,
        show: 0.2,
    };
    let mk = |t: RateTriple| HorseFactors {
        course_gate: Some(fs(t)),
        horse_surface: Some(fs(t)),
        horse_distance: Some(fs(t)),
        jockey_surface: None,
        horse_track_condition: None,
        trainer_surface: None,
        recent_form: None,
        weight_carried: None,
        jockey_recent_form: None,
        running_style: None,
    };
    let entries: Vec<_> = (1..=8)
        .map(|i| {
            let t = if i == 1 { strong } else { weak };
            (make_entry(i, &format!("ウマ{i}")), mk(t))
        })
        .collect();
    let base = estimate_probabilities(&entries);
    let compressed = estimate_probabilities_with_config(
        &entries,
        &EstimationConfig {
            place_show_power: Some(0.5),
            ..EstimationConfig::default()
        },
    );
    // 本命（idx 0）の place/show は下がり、人気薄（idx 1）は上がる（脱圧縮と逆向き）。
    assert!(
        compressed[0].place_prob < base[0].place_prob,
        "favorite place down: {} -> {}",
        base[0].place_prob,
        compressed[0].place_prob
    );
    assert!(
        compressed[1].place_prob > base[1].place_prob,
        "longshot place up: {} -> {}",
        base[1].place_prob,
        compressed[1].place_prob
    );
}

/// place/show 冪変換（#283）の中核契約: win_prob は γ に依らず**不変**。win レートを馬ごとに
/// 変えた（脱圧縮テストは全馬 win 同一で win を突き合わせていない）うえで γ=2.0 と baseline の
/// win_prob 一致を全馬アサートし、誤って win スコアに冪変換が掛かる回帰を検知する。
#[test]
fn place_show_power_leaves_win_prob_unchanged() {
    let mk = |t: RateTriple| HorseFactors {
        course_gate: Some(fs(t)),
        horse_surface: Some(fs(RateTriple::default())),
        horse_distance: Some(fs(RateTriple::default())),
        jockey_surface: None,
        horse_track_condition: None,
        trainer_surface: None,
        recent_form: None,
        weight_carried: None,
        jockey_recent_form: None,
        running_style: None,
    };
    // win も place/show も馬ごとに変える（win 0.05〜0.40 / place・show も傾斜）。place/show が
    // 馬間で異なるので γ=2.0 が分布を実際にシャープ化する（クランプを避ける程度の小さい値）。
    let entries: Vec<_> = (1..=8)
        .map(|i| {
            let f = i as f64;
            let t = RateTriple {
                win: 0.05 * f,
                place: 0.05 + 0.01 * f,
                show: 0.08 + 0.01 * f,
            };
            (make_entry(i, &format!("ウマ{i}")), mk(t))
        })
        .collect();
    let base = estimate_probabilities(&entries);
    let powered = estimate_probabilities_with_config(
        &entries,
        &EstimationConfig {
            place_show_power: Some(2.0),
            ..EstimationConfig::default()
        },
    );
    for (b, p) in base.iter().zip(&powered) {
        approx(b.win_prob, p.win_prob);
    }
    // place/show は実際に変わっている（テストが no-op を誤検証していないことの確認）。
    assert!(
        powered
            .iter()
            .zip(&base)
            .any(|(p, b)| (p.place_prob - b.place_prob).abs() > 1e-9),
        "place_show_power=2.0 で place/show は変化するはず"
    );
}

/// place/show 冪変換（#283）: 不正 γ（非有限 / ≤0）はライブラリ防御で no-op になる契約を固定する。
/// CLI は同条件を usage エラーで弾くが、ライブラリ層は黙って no-op（doc の非対称契約）。
#[test]
fn place_show_power_invalid_gamma_is_noop() {
    let mk = |t: RateTriple| HorseFactors {
        course_gate: Some(fs(t)),
        horse_surface: Some(fs(RateTriple::default())),
        horse_distance: Some(fs(t)),
        jockey_surface: None,
        horse_track_condition: None,
        trainer_surface: None,
        recent_form: None,
        weight_carried: None,
        jockey_recent_form: None,
        running_style: None,
    };
    let entries: Vec<_> = (1..=6)
        .map(|i| {
            let f = i as f64;
            let t = RateTriple {
                win: 0.05 * f,
                place: 0.05 + 0.02 * f,
                show: 0.08 + 0.02 * f,
            };
            (make_entry(i, &format!("ウマ{i}")), mk(t))
        })
        .collect();
    let base = estimate_probabilities(&entries);
    for g in [f64::NAN, f64::INFINITY, 0.0, -1.0] {
        let out = estimate_probabilities_with_config(
            &entries,
            &EstimationConfig {
                place_show_power: Some(g),
                ..EstimationConfig::default()
            },
        );
        for (a, b) in base.iter().zip(&out) {
            approx(a.win_prob, b.win_prob);
            approx(a.place_prob, b.place_prob);
            approx(a.show_prob, b.show_prob);
        }
    }
}

/// place/show 冪変換（#283）: 強い本命がいて上限クランプが発火しても、(a) 各確率は 1.0 以下、
/// (b) 単調性 win ≤ place ≤ show が保たれることを固定する（ADR 0047 が明記する境界挙動の回帰ロック）。
#[test]
fn place_show_power_clamps_strong_favorite() {
    let strong = RateTriple {
        win: 0.1,
        place: 0.5,
        show: 0.6,
    };
    let weak = RateTriple {
        win: 0.1,
        place: 0.1,
        show: 0.15,
    };
    let mk = |t: RateTriple| HorseFactors {
        course_gate: Some(fs(t)),
        horse_surface: Some(fs(t)),
        horse_distance: Some(fs(t)),
        jockey_surface: None,
        horse_track_condition: None,
        trainer_surface: None,
        recent_form: None,
        weight_carried: None,
        jockey_recent_form: None,
        running_style: None,
    };
    let entries: Vec<_> = (1..=6)
        .map(|i| {
            let t = if i == 1 { strong } else { weak };
            (make_entry(i, &format!("ウマ{i}")), mk(t))
        })
        .collect();
    let powered = estimate_probabilities_with_config(
        &entries,
        &EstimationConfig {
            place_show_power: Some(2.0),
            ..EstimationConfig::default()
        },
    );
    // 本命の place/show は冪シャープ化＋正規化で 1.0 に張り付く（クランプ発火）。
    approx(powered[0].place_prob, 1.0);
    approx(powered[0].show_prob, 1.0);
    for p in &powered {
        assert!((0.0..=1.0).contains(&p.place_prob), "place ≤ 1.0: {p:?}");
        assert!((0.0..=1.0).contains(&p.show_prob), "show ≤ 1.0: {p:?}");
        assert!(
            p.win_prob <= p.place_prob && p.place_prob <= p.show_prob,
            "monotonic: {p:?}"
        );
    }
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
        jockey_recent_form: None,
        running_style: None,
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
        jockey_recent_form: None,
        running_style: None,
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
        jockey_recent_form: None,
        running_style: None,
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
        jockey_recent_form: None,
        running_style: None,
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
        jockey_recent_form: None,
        running_style: None,
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
        jockey_recent_form: None,
        running_style: None,
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
                jockey_recent_form: None,
                running_style: None,
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
                jockey_recent_form: None,
                running_style: None,
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
        jockey_recent_form: None,
        running_style: None,
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
        recent_form_weight: None,
        trend_n: 1,
        jockey_recent_form_weight: None,
        running_style_weight: None,
        win_power: None,
        place_show_power: None,
        impute_missing_factors: false,
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
        jockey_recent_form: None,
        running_style: None,
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
        jockey_recent_form: None,
        running_style: None,
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

// ---- 騎手直近フォーム（#221） ----

#[test]
fn jockey_recent_form_score_empty_is_none() {
    assert!(jockey_recent_form_score(&[]).is_none());
}

#[test]
fn jockey_recent_form_score_all_missing_fields_is_none() {
    use super::model::JockeyFormRun;
    let runs = vec![
        JockeyFormRun {
            finishing_position: None,
            popularity: Some(3),
        },
        JockeyFormRun {
            finishing_position: Some(1),
            popularity: None,
        },
        JockeyFormRun {
            finishing_position: None,
            popularity: None,
        },
    ];
    assert!(jockey_recent_form_score(&runs).is_none());
}

#[test]
fn jockey_recent_form_score_pop_equals_pos_is_neutral() {
    use super::model::JockeyFormRun;
    // pop=pos → gap=0 → signal=0.5
    let runs = vec![JockeyFormRun {
        finishing_position: Some(3),
        popularity: Some(3),
    }];
    let score = jockey_recent_form_score(&runs).expect("some");
    assert!((score - 0.5).abs() < 1e-12, "score={score}");
}

#[test]
fn jockey_recent_form_score_surprise_win_clamps_to_one() {
    use super::model::JockeyFormRun;
    // 10 番人気 1 着: gap=9 → 0.5+9*0.08=1.22 → clamp=1.0
    let runs = vec![JockeyFormRun {
        finishing_position: Some(1),
        popularity: Some(10),
    }];
    let score = jockey_recent_form_score(&runs).expect("some");
    assert!((score - 1.0).abs() < 1e-12, "score={score}");
}

#[test]
fn jockey_recent_form_score_heavy_loss_clamps_to_zero() {
    use super::model::JockeyFormRun;
    // 1 番人気 10 着: gap=-9 → 0.5-9*0.08=-0.22 → clamp=0.0
    let runs = vec![JockeyFormRun {
        finishing_position: Some(10),
        popularity: Some(1),
    }];
    let score = jockey_recent_form_score(&runs).expect("some");
    assert!((score - 0.0).abs() < 1e-12, "score={score}");
}

#[test]
fn jockey_recent_form_score_averages_multiple_runs() {
    use super::model::JockeyFormRun;
    // 3 走: signal = 1.0 + 0.5 + 0.0 → 平均 0.5
    let runs = vec![
        JockeyFormRun {
            finishing_position: Some(1),
            popularity: Some(10),
        }, // clamp 1.0
        JockeyFormRun {
            finishing_position: Some(3),
            popularity: Some(3),
        }, // 0.5
        JockeyFormRun {
            finishing_position: Some(10),
            popularity: Some(1),
        }, // clamp 0.0
    ];
    let score = jockey_recent_form_score(&runs).expect("some");
    assert!((score - 0.5).abs() < 1e-12, "score={score}");
}

#[test]
fn jockey_recent_form_score_excludes_partial_missing_runs_from_average() {
    use super::model::JockeyFormRun;
    // 3 走中、着順 or 人気が欠落した 2 走は母数から除外され、有効な 1 走（10人気1着→clamp 1.0）
    // のみで平均される。欠落走を 0.5 等で埋めないことを固定する。
    let runs = vec![
        JockeyFormRun {
            finishing_position: Some(1),
            popularity: Some(10),
        }, // 有効: clamp 1.0
        JockeyFormRun {
            finishing_position: None,
            popularity: Some(3),
        }, // 着順欠落 → 除外
        JockeyFormRun {
            finishing_position: Some(5),
            popularity: None,
        }, // 人気欠落 → 除外
    ];
    let score = jockey_recent_form_score(&runs).expect("有効走が 1 件あるので Some");
    assert!(
        (score - 1.0).abs() < 1e-12,
        "欠落走を除外し有効 1 走のみで平均: score={score}"
    );
}

#[test]
fn jockey_recent_form_none_excluded_from_raw_score() {
    // jockey_recent_form=None の馬は項が母数から落ちてスコアが変わらない。
    let base = zero_factors();
    let with_jrf = HorseFactors {
        jockey_recent_form: Some(0.5),
        running_style: None,
        ..base.clone()
    };
    // config 重み未指定（None）→ 本番の JOCKEY_RECENT_FORM_WEIGHT 定数を使う。
    let cfg = EstimationConfig::default();
    let s_none = raw_score(&base, |r| r.win, &cfg);
    let s_mid = raw_score(&with_jrf, |r| r.win, &cfg);
    // Some(0.5) の寄与は本番 weight に依存する。weight=0.0（棄却・無効, ADR 0038）のときは
    // 分子・分母とも寄与せず None と同値。weight>0（将来再評価で有効化）のときは rate=0 の
    // zero_factors では中立 0.5 でも重み付き平均が動く。const 値に依存せず両ケースを固定する。
    if JOCKEY_RECENT_FORM_WEIGHT > 0.0 {
        assert!(
            s_mid != s_none,
            "weight>0 では中立 0.5 でも rate=0 なら変化する: none={s_none}, mid={s_mid}"
        );
    } else {
        approx(s_mid, s_none);
    }

    // config で weight を明示的に与えれば、定数値に関係なく寄与する（sweep フラグの検証）。
    let cfg_w = EstimationConfig {
        jockey_recent_form_weight: Some(0.25),
        running_style_weight: None,
        ..EstimationConfig::default()
    };
    assert!(
        raw_score(&with_jrf, |r| r.win, &cfg_w) != s_none,
        "config 重み 0.25 を与えれば中立 0.5 でもスコアが動く"
    );

    // None は母数から除外 → zero_factors と同一スコア（weight 値によらず常に成立）。
    let none_jrf = HorseFactors {
        jockey_recent_form: None,
        running_style: None,
        ..base
    };
    approx(raw_score(&none_jrf, |r| r.win, &cfg), s_none);
}

/// 本番 predict（`predict_race`）が使う `production()` の設定を固定する回帰ガード。
/// 縮約 m を取り違えたり recency を誤って有効化すると CI で検知する（#75/ADR 0016）。
/// win_power（#246/ADR 0042）γ=1.25・place_show_power（#283/ADR 0047）γ=2.0 も採用値を固定する。
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
    assert_eq!(
        c.win_power
            .expect("production は win_power on（#246/ADR 0042）"),
        RECOMMENDED_WIN_POWER
    );
    assert!((RECOMMENDED_WIN_POWER - 1.25).abs() < 1e-12);
    assert_eq!(
        c.place_show_power
            .expect("production は place_show_power on（#283/ADR 0047）"),
        RECOMMENDED_PLACE_SHOW_POWER
    );
    assert!((RECOMMENDED_PLACE_SHOW_POWER - 2.0).abs() < 1e-12);
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
fn win_power_gamma_one_is_noop() {
    // γ=1.0 は実質恒等。win/place/show すべて不変。
    let probs = vec![
        prob(1, 0.6, 0.7, 0.8),
        prob(2, 0.3, 0.5, 0.6),
        prob(3, 0.1, 0.2, 0.4),
    ];
    let out = apply_win_power(&probs, 1.0);
    for (a, b) in probs.iter().zip(&out) {
        approx(a.win_prob, b.win_prob);
        approx(a.place_prob, b.place_prob);
        approx(a.show_prob, b.show_prob);
    }
}

#[test]
fn win_power_non_finite_or_nonpositive_is_noop() {
    let probs = vec![prob(1, 0.6, 0.7, 0.8), prob(2, 0.4, 0.5, 0.6)];
    for g in [f64::NAN, f64::INFINITY, 0.0, -1.0] {
        let out = apply_win_power(&probs, g);
        approx(out[0].win_prob, 0.6);
        approx(out[1].win_prob, 0.4);
    }
}

#[test]
fn win_power_shifts_mass_to_favorite() {
    // γ>1 で本命の win が増え穴の win が減る。合計は 1.0 を保つ。
    let probs = vec![
        prob(1, 0.5, 0.5, 0.5),
        prob(2, 0.3, 0.3, 0.3),
        prob(3, 0.2, 0.2, 0.2),
    ];
    let out = apply_win_power(&probs, 2.0);
    assert!(out[0].win_prob > 0.5, "favorite up: {}", out[0].win_prob);
    assert!(out[2].win_prob < 0.2, "longshot down: {}", out[2].win_prob);
    let total: f64 = out.iter().map(|p| p.win_prob).sum();
    approx(total, 1.0);
}

#[test]
fn win_power_preserves_monotonicity() {
    // 冪変換で favorite の win が元の place を超えても win ≤ place ≤ show を再是正する。
    let probs = vec![
        prob(1, 0.8, 0.81, 0.82),
        prob(2, 0.15, 0.5, 0.6),
        prob(3, 0.05, 0.3, 0.5),
    ];
    let out = apply_win_power(&probs, 3.0);
    for p in &out {
        assert!(
            p.win_prob <= p.place_prob && p.place_prob <= p.show_prob,
            "{p:?}"
        );
        assert!((0.0..=1.0).contains(&p.win_prob) && (0.0..=1.0).contains(&p.show_prob));
    }
}

#[test]
fn win_power_empty_is_empty() {
    let out = apply_win_power(&[], 2.0);
    assert!(out.is_empty());
}

#[test]
fn win_power_all_zero_win_is_noop() {
    // 全馬 win_prob=0（total<=0 ガード）→ 入力をそのまま返す。
    let probs = vec![prob(1, 0.0, 0.3, 0.5), prob(2, 0.0, 0.2, 0.4)];
    let out = apply_win_power(&probs, 2.0);
    for (a, b) in probs.iter().zip(&out) {
        approx(a.win_prob, b.win_prob);
        approx(a.place_prob, b.place_prob);
        approx(a.show_prob, b.show_prob);
    }
}

#[test]
fn win_power_gamma_below_one_spreads_to_longshots() {
    // γ<1（逆方向 sweep。コードは許可）で本命の win が下がり穴の win が上がる。合計1.0維持。
    let probs = vec![
        prob(1, 0.6, 0.6, 0.6),
        prob(2, 0.3, 0.3, 0.3),
        prob(3, 0.1, 0.1, 0.1),
    ];
    let out = apply_win_power(&probs, 0.5);
    assert!(out[0].win_prob < 0.6, "favorite down: {}", out[0].win_prob);
    assert!(out[2].win_prob > 0.1, "longshot up: {}", out[2].win_prob);
    let total: f64 = out.iter().map(|p| p.win_prob).sum();
    approx(total, 1.0);
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

// ---- 欠落 stat factor の field mean 補完（#272 改善② / ADR 0057） ----

/// horse_surface だけ設定できる（他 stat/scalar は欠落）テスト用 HorseFactors。
fn surf(win: Option<f64>) -> HorseFactors {
    HorseFactors {
        course_gate: None,
        horse_surface: win.map(|w| {
            fs(RateTriple {
                win: w,
                place: w,
                show: w,
            })
        }),
        horse_distance: None,
        jockey_surface: None,
        trainer_surface: None,
        horse_track_condition: None,
        recent_form: None,
        weight_carried: None,
        jockey_recent_form: None,
        running_style: None,
    }
}

#[test]
fn field_impute_uses_present_mean_and_prior_fallback() {
    // 縮約 off（default）では factor_value = 生レート。present 2 頭（A,B）の horse_surface 平均 = 0.5。
    let factors = [surf(Some(0.4)), surf(Some(0.6)), surf(None)];
    let imp = FactorImpute::from_field(factors.iter(), |r| r.win, &EstimationConfig::default());
    approx(imp.horse_surface.unwrap(), 0.5);
    // present 0 頭の factor は prior へフォールバック（レース内平均が立たない）。
    approx(imp.course_gate.unwrap(), PRIOR_RATE.win);
    approx(imp.jockey_surface.unwrap(), PRIOR_RATE.win);

    // present が 1 頭だと平均が単一馬に潰れて中立にならないため prior で埋める。
    let one = [surf(Some(0.6)), surf(None)];
    let imp1 = FactorImpute::from_field(one.iter(), |r| r.win, &EstimationConfig::default());
    approx(imp1.horse_surface.unwrap(), PRIOR_RATE.win);
}

#[test]
fn impute_fills_missing_factor_instead_of_dropping() {
    // course_gate のみ持ち horse_surface を欠く馬。
    let mut f = surf(None);
    f.course_gate = Some(fs(RateTriple {
        win: 0.2,
        place: 0.2,
        show: 0.2,
    }));
    let cfg = EstimationConfig::default();
    // drop（従来）: 母数は course_gate のみ → 0.2。
    let dropped = raw_score(&f, |r| r.win, &cfg);
    approx(dropped, 0.2);
    // horse_surface を field mean 0.8 で補完すると course(0.2) と 0.8 の重み付き平均へ動く。
    let imp = FactorImpute {
        horse_surface: Some(0.8),
        ..FactorImpute::DROP
    };
    let filled = raw_score_with_impute(&f, |r| r.win, &cfg, &imp);
    assert!(
        filled > dropped && filled < 0.8,
        "filled={filled} は drop(0.2) と impute(0.8) の間に入るはず"
    );
    // 全 drop 補完は raw_score（従来）と厳密一致（回帰ガード）。
    approx(
        raw_score_with_impute(&f, |r| r.win, &cfg, &FactorImpute::DROP),
        dropped,
    );
}

#[test]
fn estimate_imputation_changes_missing_horse_probability() {
    // A,B は horse_surface を持ち field mean が立つ。C は欠落し、course_gate を field mean と別値に
    // することで補完 on/off で C の勝率が動く（配線の回帰ガード）。
    let mk = |surface: Option<f64>, cg: f64| {
        let mut f = surf(surface);
        f.course_gate = Some(fs(RateTriple {
            win: cg,
            place: cg,
            show: cg,
        }));
        f
    };
    let entries = vec![
        (make_entry(1, "A"), mk(Some(0.7), 0.5)),
        (make_entry(2, "B"), mk(Some(0.3), 0.5)),
        (make_entry(3, "C"), mk(None, 0.2)),
    ];
    let off = estimate_probabilities_with_config(&entries, &EstimationConfig::default());
    let impute_cfg = EstimationConfig {
        impute_missing_factors: true,
        ..EstimationConfig::default()
    };
    let on = estimate_probabilities_with_config(&entries, &impute_cfg);
    let c = HorseNum::try_from(3u32).unwrap();
    let c_off = off.iter().find(|p| p.horse_num == c).unwrap().win_prob;
    let c_on = on.iter().find(|p| p.horse_num == c).unwrap().win_prob;
    assert!(
        (c_off - c_on).abs() > 1e-6,
        "補完で C の勝率が動くはず: off={c_off} on={c_on}"
    );
    // 補完 off は現行の drop 経路と一致（単調性も維持）。
    for p in &on {
        assert!(p.win_prob <= p.place_prob && p.place_prob <= p.show_prob);
    }
}

#[test]
fn field_impute_is_per_selector() {
    // win/place/show でレートが異なる horse_surface。selector ごとに独立に field mean を取り、
    // present 0 頭の factor は selector 対応の PRIOR_RATE でフォールバックすることを確認する。
    let hs = |w: f64, p: f64, s: f64| HorseFactors {
        horse_surface: Some(fs(RateTriple {
            win: w,
            place: p,
            show: s,
        })),
        ..surf(None)
    };
    let factors = [hs(0.4, 0.5, 0.6), hs(0.6, 0.7, 0.8), surf(None)];
    let cfg = EstimationConfig::default();
    // horse_surface: present 2 頭の平均は selector 別（win 0.5 / place 0.6 / show 0.7）。
    approx(
        FactorImpute::from_field(factors.iter(), |r| r.win, &cfg)
            .horse_surface
            .unwrap(),
        0.5,
    );
    approx(
        FactorImpute::from_field(factors.iter(), |r| r.place, &cfg)
            .horse_surface
            .unwrap(),
        0.6,
    );
    approx(
        FactorImpute::from_field(factors.iter(), |r| r.show, &cfg)
            .horse_surface
            .unwrap(),
        0.7,
    );
    // present 0 頭の course_gate は selector 別 prior（place/show は win と別値）。
    approx(
        FactorImpute::from_field(factors.iter(), |r| r.place, &cfg)
            .course_gate
            .unwrap(),
        PRIOR_RATE.place,
    );
    approx(
        FactorImpute::from_field(factors.iter(), |r| r.show, &cfg)
            .course_gate
            .unwrap(),
        PRIOR_RATE.show,
    );
}

#[test]
fn all_factors_missing_horse_imputes_to_weight_nonzero() {
    // 全 factor 欠落の馬。drop なら weight==0 → score 0.0（均等フォールバック, ADR 0014）。
    let bare = surf(None);
    let cfg = EstimationConfig::default();
    approx(raw_score(&bare, |r| r.win, &cfg), 0.0);
    // 補完有効時は全 stat factor が field mean（present 0 → prior）で Some 化され weight>0 になり、
    // weight==0 の均等フォールバック経路は該当馬で非到達になる（#272 改善② の意図した挙動変化）。
    let prior = PRIOR_RATE.win;
    let imp = FactorImpute {
        course_gate: Some(prior),
        horse_surface: Some(prior),
        horse_distance: Some(prior),
        jockey_surface: Some(prior),
        trainer_surface: Some(prior),
        horse_track_condition: Some(prior),
    };
    approx(raw_score_with_impute(&bare, |r| r.win, &cfg, &imp), prior);
}

#[test]
fn production_enables_imputation_default_disables() {
    assert!(
        EstimationConfig::production().impute_missing_factors,
        "predict 本番は欠落補完を有効にする（ADR 0057）"
    );
    assert!(
        !EstimationConfig::default().impute_missing_factors,
        "Default は後方互換で drop（補完なし）"
    );
}

#[test]
fn field_impute_mean_uses_shrunk_rate_under_shrinkage() {
    // production は shrinkage m=10 を通す。field mean は「present 馬の縮約後レート平均」であり、
    // 生レート平均ではないことを直接ガードする（factor_value と補完の縮約整合の回帰防止）。
    let m = 10.0;
    let factors = [surf(Some(0.4)), surf(Some(0.6))]; // starts=10（fs のデフォルト）
    let cfg = shrink_cfg(m);
    let expected =
        (shrink_rate(0.4, 10, PRIOR_RATE.win, m) + shrink_rate(0.6, 10, PRIOR_RATE.win, m)) / 2.0;
    let got = FactorImpute::from_field(factors.iter(), |r| r.win, &cfg)
        .horse_surface
        .unwrap();
    approx(got, expected);
    // 縮約により生レート平均 0.5 とは一致しない（縮約が実際に効いている証拠）。
    assert!(
        (got - 0.5).abs() > 1e-6,
        "縮約後平均は生平均 0.5 と異なるはず: {got}"
    );
}

// ---- 脚質（先行度）導出（#329 Phase1） ----

#[test]
fn parse_corner_positions_splits_and_filters() {
    assert_eq!(parse_corner_positions("10-9-5-5"), vec![10, 9, 5, 5]);
    assert_eq!(parse_corner_positions("3-3"), vec![3, 3]);
    // 空・非数値・0（順位は 1 起点）は除外する。
    assert_eq!(parse_corner_positions(""), Vec::<u32>::new());
    assert_eq!(parse_corner_positions("-"), Vec::<u32>::new());
    assert_eq!(parse_corner_positions("1--2"), vec![1, 2]);
    assert_eq!(parse_corner_positions("0-2"), vec![2]);
    assert_eq!(parse_corner_positions("a-2"), vec![2]);
}

#[test]
fn leading_position_relativizes_first_corner_by_field_size() {
    // 16 頭立て 1 番手 = 逃げ（先行度 1.0）、16 番手 = 追込（0.0）、中団はその間。
    approx(leading_position(&[1, 3, 5], 16).unwrap(), 1.0);
    approx(leading_position(&[16], 16).unwrap(), 0.0);
    // 8 頭立て 3 番手: rel=(3-1)/(8-1)=2/7、先行度=1-2/7。
    approx(leading_position(&[3], 8).unwrap(), 1.0 - 2.0 / 7.0);
    // 同じ 3 番手でも頭数が違えば先行度が変わる（相対化が効いている）。
    assert!(leading_position(&[3], 16).unwrap() > leading_position(&[3], 8).unwrap());
}

#[test]
fn leading_position_none_on_invalid_inputs() {
    assert!(leading_position(&[], 16).is_none()); // 通過順位なし
    assert!(leading_position(&[1], 1).is_none()); // 頭数 < 2（正規化不能）
    assert!(leading_position(&[0], 16).is_none()); // 順位 0（1 起点外）
    assert!(leading_position(&[17], 16).is_none()); // 順位が頭数超過（データ不整合）
}

#[test]
fn running_style_of_run_requires_corner_and_field_size() {
    // corner・頭数が揃えば先頭コーナーの先行度を返す。
    approx(running_style_of_run(Some("1-1-1"), Some(16)).unwrap(), 1.0);
    // どちらか欠けたら None（母数除外）。
    assert!(running_style_of_run(None, Some(16)).is_none());
    assert!(running_style_of_run(Some("3-3"), None).is_none());
    // 解釈不能な corner も None。
    assert!(running_style_of_run(Some(""), Some(16)).is_none());
}

#[test]
fn running_style_zero_weight_does_not_change_score() {
    // 本番 RUNNING_STYLE_WEIGHT=0.0（override なし）では running_style の有無でスコアが不変
    // （measure-first・dump 列のみ・production 挙動不変の回帰ガード）。
    // 非ゼロレートの実 factor を持つ base（score != 0）で、running_style 付与がスコアを
    // 一切動かさないことを確認する（0 レート base だと不変が自明化するため）。
    let cfg = EstimationConfig::default();
    let mut base = zero_factors();
    base.course_gate = Some(fs(RateTriple {
        win: 0.3,
        place: 0.4,
        show: 0.5,
    }));
    let baseline = raw_score(&base, |r| r.win, &cfg);
    assert!(baseline > 0.0, "base は非ゼロスコアであること: {baseline}");
    let mut with_rs = base.clone();
    with_rs.running_style = Some(0.9);
    approx(raw_score(&with_rs, |r| r.win, &cfg), baseline);
}

#[test]
fn running_style_weight_override_moves_score() {
    // sweep 用 override（config.running_style_weight）を効かせると running_style がスコアに寄与する。
    let base = zero_factors();
    let mut with_rs = zero_factors();
    with_rs.running_style = Some(1.0);
    let cfg = EstimationConfig {
        running_style_weight: Some(1.0),
        ..EstimationConfig::default()
    };
    assert!(
        raw_score(&with_rs, |r| r.win, &cfg) > raw_score(&base, |r| r.win, &cfg),
        "override 有効時は先行度 1.0 がスコアを押し上げるはず"
    );
    // None の馬は override 有効でも母数から落ちる（zero_factors と一致）。
    approx(
        raw_score(&base, |r| r.win, &cfg),
        raw_score(&zero_factors(), |r| r.win, &EstimationConfig::default()),
    );
}
