use std::collections::HashMap;

use chrono::NaiveDate;

use crate::horse_result::{HorseName, HorseNum, HorseResult};
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
    /// 前走フォーム [0,1]（0.5=中立）。前走が無い／有効な signal が無い馬は `None`。
    /// win/place/show に同値で寄与する（フォームは方向に依らず全体を底上げ／押し下げる）。
    pub recent_form: Option<f64>,
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

/// 単勝確率を市場オッズ（単勝）の implied 確率とブレンドする（#72）。
///
/// `market_win_odds` は馬番→単勝確定オッズ（払戻倍率, ≥1.0）。各馬の implied 確率
/// `1/odds` をレース内で合計 1.0 に正規化（控除率＝オーバーラウンドを除去）し、モデルの
/// `win_prob` と `alpha`（モデル重み, `1-alpha` が市場重み）で線形ブレンドする。`alpha >= 1.0`
/// またはオッズが空のときはモデル確率をそのまま返す（no-op）。オッズの無い馬はブレンド時点では
/// モデル値を保つ（最後の win 合計 1.0 再正規化で全体と同じ係数でスケールはされる）。
///
/// ブレンドで win が動くため、最後に win 合計を 1.0 へ再正規化し、`place`/`show` は
/// `win ≤ place ≤ show` を保つよう累積 max で再是正する（v1 は win のみブレンド対象で
/// place/show のレートはモデル値を踏襲する）。
///
/// 前提・既知の割り切り（v1）:
/// - **(ほぼ)全頭のオッズが揃っていることを前提**とする。implied の正規化母数はオッズを持つ馬
///   のみの合計なので、一部の馬しかオッズが無い部分カバレッジでは市場重み `(1-α)` がカバー済みの
///   少数馬に偏って乗り、過大評価になりうる。実運用の単勝オッズは全頭分そろうため通常は問題ない。
/// - place/show は単調再是正のみで、**場内合計（2.0/3.0）は再正規化しない**ため、ブレンド後は
///   その合計が崩れうる。place/show の精密なブレンドは将来課題。
pub fn blend_with_market_win(
    probs: &[HorseProbability],
    market_win_odds: &HashMap<HorseNum, f64>,
    alpha: f64,
) -> Vec<HorseProbability> {
    // 非有限な α（NaN 等）は no-op 扱い（呼び出し側で検証済みだが防御的に弾く）。
    if !alpha.is_finite() {
        return probs.to_vec();
    }
    let alpha = alpha.clamp(0.0, 1.0);
    if probs.is_empty() || market_win_odds.is_empty() || alpha >= 1.0 {
        return probs.to_vec();
    }

    // 市場 implied 確率: 1/odds を合計 1.0 に正規化（オッズのある馬のみが母数）。
    // 単勝オッズ（払戻倍率）は ≥1.0。型検証を経ていない生の f64（backtest が results.odds から渡す
    // 経路）に異常値が混じっても弾けるよう doc 契約どおり `>= 1.0` でフィルタする。OddsValue 由来の
    // 経路では常に満たすが、フォールバック経路のための防御。
    let implied: HashMap<HorseNum, f64> = market_win_odds
        .iter()
        .filter(|&(_, &odds)| odds.is_finite() && odds >= 1.0)
        .map(|(&num, &odds)| (num, 1.0 / odds))
        .collect();
    let overround: f64 = implied.values().sum();
    if overround <= 0.0 {
        return probs.to_vec();
    }

    // モデル win と市場 implied をブレンド（オッズの無い馬はモデル値のまま）。
    let blended: Vec<f64> = probs
        .iter()
        .map(|p| match implied.get(&p.horse_num) {
            Some(&imp) => alpha * p.win_prob + (1.0 - alpha) * (imp / overround),
            None => p.win_prob,
        })
        .collect();

    // 部分カバレッジや凸結合のドリフトを吸収して win 合計を 1.0 へ戻す。
    // `min(1.0)` は w ≤ total（全要素非負）より数学的には恒等だが、浮動小数点の保険として残す。
    let total: f64 = blended.iter().sum();
    let win_probs: Vec<f64> = if total > 0.0 {
        blended.iter().map(|w| (w / total).min(1.0)).collect()
    } else {
        blended
    };

    probs
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let win = win_probs[i];
            let place = p.place_prob.max(win).min(1.0);
            let show = p.show_prob.max(place).min(1.0);
            HorseProbability {
                horse_num: p.horse_num,
                horse_name: p.horse_name.clone(),
                win_prob: win,
                place_prob: place,
                show_prob: show,
            }
        })
        .collect()
}

const COURSE_GATE_WEIGHT: f64 = 2.0;
const SURFACE_WEIGHT: f64 = 1.0;
const DISTANCE_WEIGHT: f64 = 1.0;
const JOCKEY_WEIGHT: f64 = 1.0;
/// 前走フォーム項の重み。#30 バックテストで検証して決定（ADR 0009）。
const FORM_WEIGHT: f64 = 0.25;

/// 存在する factor の**重み付き平均**を返す。騎手未登録馬・前走なし馬はその項と重みを母数から
/// 除外して評価するため、欠落項で不当に減点されない（ADR 0007/0008）。全馬が同条件のときは
/// 定数除算となり、レース内正規化後の相対順位は変わらない。
///
/// `recent_form` はスカラー（[0,1]、0.5=中立）で win/place/show に同値で寄与する。
fn raw_score(factors: &HorseFactors, rate: fn(&RateTriple) -> f64) -> f64 {
    let mut weighted = COURSE_GATE_WEIGHT * rate(&factors.course_gate)
        + SURFACE_WEIGHT * rate(&factors.horse_surface)
        + DISTANCE_WEIGHT * rate(&factors.horse_distance);
    let mut weight = COURSE_GATE_WEIGHT + SURFACE_WEIGHT + DISTANCE_WEIGHT;
    if let Some(jockey) = factors.jockey_surface {
        weighted += JOCKEY_WEIGHT * rate(&jockey);
        weight += JOCKEY_WEIGHT;
    }
    if let Some(form) = factors.recent_form {
        weighted += FORM_WEIGHT * form;
        weight += FORM_WEIGHT;
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

/// 馬体重変化がこの kg を超えると不安定として最低評価（0）にする。
const WEIGHT_CHANGE_CAP: f64 = 20.0;
/// 前走の人気順位と着順の差 1 つあたりのスコア寄与。
const POP_GAP_K: f64 = 0.08;

/// 直近 1 走（`prev`、その開催日 `prev_date`）と対象レース日 `race_date` から「前走フォーム」
/// スコア `[0,1]`（0.5=中立）を算出する。利用できる sub-signal（馬体重変化・前走人気乖離・前走間隔）の
/// 平均を返す。有効な signal が 1 つも無い場合は `None`（前走情報が乏しい→スコアに寄与させない）。
pub fn recent_form_score(
    prev: &HorseResult,
    prev_date: NaiveDate,
    race_date: NaiveDate,
) -> Option<f64> {
    let mut signals: Vec<f64> = Vec::new();

    // 馬体重変化: |Δkg| が小さいほど安定＝良。CAP 超で 0。
    if let Some(dw) = prev.weight_change {
        signals.push(1.0 - (dw.unsigned_abs() as f64 / WEIGHT_CHANGE_CAP).min(1.0));
    }

    // 前走人気乖離: 人気順位より好走（着順が人気順位より小さい）で加点、凡走で減点。
    // 着順なし（中止・失格・取消で finishing_position = None）の前走は乖離を測れないため、
    // この signal を落として残りの signal（体重・間隔）で評価する。
    if let (Some(pop), Some(pos)) = (prev.popularity, prev.finishing_position.map(|p| p.value())) {
        let gap = pop as f64 - pos as f64; // >0: 人気以上の好走
        signals.push((0.5 + gap * POP_GAP_K).clamp(0.0, 1.0));
    }

    // 前走間隔: 中2週(14)〜2ヶ月(60)を最適(1.0)、連闘(<14)/長休(>120)を逓減。
    // 本番経路では find_recent_runs が `races.date < before` で前走のみ返すため days は常に正。
    // `days > 0` は異常データ（同日/未来の前走）に対する防御で、その場合は間隔 signal を落とす。
    let days = (race_date - prev_date).num_days();
    if days > 0 {
        signals.push(interval_form(days));
    }

    if signals.is_empty() {
        None
    } else {
        Some(signals.iter().sum::<f64>() / signals.len() as f64)
    }
}

/// 前走間隔（日数）→ `[0,1]` の台形マップ。区分は離散で、境界に小さな段差がある（heuristic）。
fn interval_form(days: i64) -> f64 {
    match days {
        d if d <= 7 => 0.3,                                  // 連闘・中1週未満
        d if d < 14 => 0.3 + 0.7 * (d - 7) as f64 / 7.0,     // 8〜13 日: 0.3→0.9 にランプ
        d if d <= 60 => 1.0,                                 // 中2週〜2ヶ月: 最適帯
        d if d <= 120 => 1.0 - 0.5 * (d - 60) as f64 / 60.0, // 60→120 日で 1.0→0.5
        _ => 0.5,                                            // 長期休み明け（不確実）
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horse_result::{FinishingPosition, GateNum, HorseName, HorseNum, ResultStatus};
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
        }
    }

    fn zero_factors() -> HorseFactors {
        HorseFactors {
            course_gate: RateTriple::default(),
            horse_surface: RateTriple::default(),
            horse_distance: RateTriple::default(),
            jockey_surface: None,
            recent_form: None,
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
                    recent_form: None,
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
                    recent_form: None,
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
            recent_form: None,
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
            recent_form: None,
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
            recent_form: None,
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
            recent_form: None,
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
            recent_form: None,
        };
        let without_jockey = HorseFactors {
            course_gate: base,
            horse_surface: base,
            horse_distance: base,
            jockey_surface: None,
            recent_form: None,
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
            recent_form: None,
            ..with_equal_jockey
        };
        assert!(raw_score(&strong, |r| r.win) > s_without);
        assert!(raw_score(&weak, |r| r.win) < s_without);
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
            assert!(p.win_prob <= p.place_prob && p.place_prob <= p.show_prob, "{p:?}");
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
        assert!(recent_form_score(&prev, ymd(2026, 5, 1), ymd(2026, 5, 1)).is_none());
    }

    #[test]
    fn recent_form_weight_change_smaller_is_better() {
        let d = ymd(2026, 5, 1);
        let pd = ymd(2026, 4, 1); // 30 日前（最適帯 1.0）
        // 体重変化のみで比較するため人気・着順は欠損。
        let stable = recent_form_score(&prev_result(Some(2), None, None), pd, d).unwrap();
        let swingy = recent_form_score(&prev_result(Some(18), None, None), pd, d).unwrap();
        assert!(stable > swingy, "stable={stable}, swingy={swingy}");
        // CAP(20kg) 超は体重 signal が 0。間隔 signal(1.0) との平均なので 0.5。
        let huge = recent_form_score(&prev_result(Some(40), None, None), pd, d).unwrap();
        assert!((huge - 0.5).abs() < 1e-9, "huge={huge}");
    }

    #[test]
    fn recent_form_popularity_gap() {
        let d = ymd(2026, 5, 1);
        let pd = ymd(2026, 4, 1);
        // 5 番人気で 2 着（人気以上に好走）→ 加点。1 番人気で 8 着（凡走）→ 減点。
        let over = recent_form_score(&prev_result(None, Some(5), Some(2)), pd, d).unwrap();
        let under = recent_form_score(&prev_result(None, Some(1), Some(8)), pd, d).unwrap();
        assert!(over > under, "over={over}, under={under}");
    }

    #[test]
    fn recent_form_interval_band() {
        // 最適帯(30日)=1.0、連闘(3日)=0.3、長休(200日)=0.5。間隔のみ（他欠損）。
        let base = ymd(2026, 5, 1);
        let optimal = recent_form_score(&prev_result(None, None, None), ymd(2026, 4, 1), base);
        let rento = recent_form_score(&prev_result(None, None, None), ymd(2026, 4, 28), base);
        let layoff = recent_form_score(&prev_result(None, None, None), ymd(2025, 10, 13), base);
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
        let f = recent_form_score(&prev_result(Some(2), Some(3), None), pd, d).unwrap();
        let weight_sig = 1.0 - 2.0 / WEIGHT_CHANGE_CAP; // 0.9
        assert!((f - (weight_sig + 1.0) / 2.0).abs() < 1e-9, "form={f}");
    }

    #[test]
    fn recent_form_in_unit_range() {
        // 全 signal が揃ったケースでも [0,1]。
        let d = ymd(2026, 5, 1);
        let pd = ymd(2026, 4, 10);
        let f = recent_form_score(&prev_result(Some(-4), Some(3), Some(1)), pd, d).unwrap();
        assert!((0.0..=1.0).contains(&f), "form={f}");
    }

    #[test]
    fn recent_form_keeps_monotonicity_in_estimate() {
        // recent_form を持つ馬・持たない馬が混在しても単調性は保たれる。
        let mut f_with = zero_factors();
        f_with.course_gate = RateTriple {
            win: 0.3,
            place: 0.4,
            show: 0.5,
        };
        f_with.recent_form = Some(0.9);
        let mut f_without = zero_factors();
        f_without.course_gate = RateTriple {
            win: 0.2,
            place: 0.3,
            show: 0.4,
        };
        let entries = vec![
            (make_entry(1, "ウマA"), f_with),
            (make_entry(2, "ウマB"), f_without),
        ];
        let probs = estimate_probabilities(&entries);
        for p in &probs {
            assert!(p.win_prob <= p.place_prob && p.place_prob <= p.show_prob);
        }
    }
}
