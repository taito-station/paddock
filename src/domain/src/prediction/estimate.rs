//! レース内の馬群から win/place/show 確率を推定し、市場オッズとブレンドする中核ロジック（#72/#75）。

use std::collections::HashMap;

use super::config::EstimationConfig;
use super::model::{HorseFactors, HorseProbability};
use super::scoring::{normalize_to_sum, raw_score};
use crate::horse_result::HorseNum;
use crate::race_card::HorseEntry;

/// 現行挙動（縮約・減衰なし）で確率推定する。既存呼び出し・テスト互換のため signature を保つ。
pub fn estimate_probabilities(entries: &[(HorseEntry, HorseFactors)]) -> Vec<HorseProbability> {
    estimate_probabilities_with_config(entries, &EstimationConfig::default())
}

/// `config` でベイズ縮約・リーセンシーの有効化を切り替えて確率推定する（#75）。
/// `EstimationConfig::default()`（両方 `None`）は [`estimate_probabilities`] と同一挙動。
pub fn estimate_probabilities_with_config(
    entries: &[(HorseEntry, HorseFactors)],
    config: &EstimationConfig,
) -> Vec<HorseProbability> {
    if entries.is_empty() {
        return Vec::new();
    }

    let win_scores: Vec<f64> = entries
        .iter()
        .map(|(_, f)| raw_score(f, |r| r.win, config))
        .collect();
    let place_scores: Vec<f64> = entries
        .iter()
        .map(|(_, f)| raw_score(f, |r| r.place, config))
        .collect();
    let show_scores: Vec<f64> = entries
        .iter()
        .map(|(_, f)| raw_score(f, |r| r.show, config))
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

/// win_prob を冪変換 `win'_i ∝ win_i^gamma` して場内合計 1.0 へ再正規化する（#246 / ADR 0042）。
///
/// Harville は IIA 的性質から人気薄馬の「1着」確率を過大評価しがちで、これが穴を絡めた馬連 EV を
/// 馬単より高く見せる一因になる。`gamma > 1.0` で人気馬の win を相対的に強調し穴の 1 着を縮約する
/// ことでこの偏りを是正する（`gamma < 1.0` は逆方向。backtest の `--win-power` で sweep するため許可）。
///
/// `gamma` が非有限 / `<= 0.0` / ちょうど `1.0`（厳密一致近傍, `< f64::EPSILON`）、または入力が空・
/// win 合計が 0 のときは no-op。production の γ は離散値（1.25 等）なのでこの厳密判定で十分。
/// win を変えると単調性 `win ≤ place ≤ show` が崩れうるため、`blend_with_market_win` 末尾と同型の
/// 累積 max で place/show を再是正する（place/show の場内合計 2.0/3.0 が崩れうる点も blend と同じ
/// 既知の割り切り。連系・着順 EV は win_prob から導くため、ここでの win 校正がそのまま反映される）。
pub fn apply_win_power(probs: &[HorseProbability], gamma: f64) -> Vec<HorseProbability> {
    if !gamma.is_finite() || gamma <= 0.0 || (gamma - 1.0).abs() < f64::EPSILON || probs.is_empty()
    {
        return probs.to_vec();
    }

    let powered: Vec<f64> = probs.iter().map(|p| p.win_prob.powf(gamma)).collect();
    let total: f64 = powered.iter().sum();
    // total<=0 は全 win 0、!finite は win_prob 不変条件（[0,1]）が崩れた場合の防御（NaN 伝播回避）。
    if total <= 0.0 || !total.is_finite() {
        return probs.to_vec();
    }

    probs
        .iter()
        .zip(powered)
        .map(|(p, w)| {
            let win = (w / total).min(1.0);
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

/// place/show を冪変換 `p'_i ∝ p_i^gamma` して場内合計 2.0 / 3.0 へ再正規化する（#286 / ADR 0047）。
///
/// m=10 縮約後の place/show は中央に圧縮（強い連対/複勝馬を過小・弱い馬を過大）するため、`gamma > 1.0`
/// で相対的にシャープ化し校正を改善する。**win は変えない**（top-1・EV・回収率は win 由来なので不変）。
/// place を ^gamma→合計 2.0、show を ^gamma→合計 3.0 に再正規化し、最後に `win ≤ place ≤ show` を
/// 累積 max で再是正する（[`apply_win_power`] と同型。cap で合計が 2.0/3.0 を下回りうる割り切りも同じ）。
/// `gamma` 非有限 / `<= 0.0` / ≈ `1.0` / 入力空、または place・show の合計が 0/非有限のときは no-op。
pub fn apply_placeshow_power(probs: &[HorseProbability], gamma: f64) -> Vec<HorseProbability> {
    if !gamma.is_finite() || gamma <= 0.0 || (gamma - 1.0).abs() < f64::EPSILON || probs.is_empty()
    {
        return probs.to_vec();
    }
    // vals を ^gamma して合計 target に再正規化（各要素 ≤1.0）。合計 0/非有限は None（no-op 判定）。
    let pow_sum = |vals: Vec<f64>, target: f64| -> Option<Vec<f64>> {
        let powered: Vec<f64> = vals.iter().map(|v| v.powf(gamma)).collect();
        let total: f64 = powered.iter().sum();
        if total <= 0.0 || !total.is_finite() {
            return None;
        }
        Some(
            powered
                .iter()
                .map(|v| (v / total * target).min(1.0))
                .collect(),
        )
    };
    let place = pow_sum(probs.iter().map(|p| p.place_prob).collect(), 2.0);
    let show = pow_sum(probs.iter().map(|p| p.show_prob).collect(), 3.0);
    let (Some(place), Some(show)) = (place, show) else {
        return probs.to_vec();
    };

    probs
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let win = p.win_prob;
            let place = place[i].max(win).min(1.0);
            let show = show[i].max(place).min(1.0);
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

#[cfg(test)]
mod placeshow_power_tests {
    use super::*;
    use crate::horse_result::{HorseName, HorseNum};

    fn prob(n: u32, w: f64, p: f64, s: f64) -> HorseProbability {
        HorseProbability {
            horse_num: HorseNum::try_from(n).unwrap(),
            horse_name: HorseName::try_from(format!("ウマ{n}")).unwrap(),
            win_prob: w,
            place_prob: p,
            show_prob: s,
        }
    }

    #[test]
    fn powers_placeshow_with_concrete_values_and_keeps_win() {
        // 2 頭・γ=2。place^2→合計2.0 / show^2→合計3.0 へ再正規化、win は不変、単調 win≤place≤show。
        let probs = vec![prob(1, 0.3, 0.4, 0.5), prob(2, 0.1, 0.2, 0.3)];
        let out = apply_placeshow_power(&probs, 2.0);
        // win は完全不変。
        assert_eq!(out[0].win_prob, 0.3);
        assert_eq!(out[1].win_prob, 0.1);
        // h2: place .2^2/(.2^2+.4^2)*2 = .04/.20*2 = 0.4、show .3^2/(.3^2+.5^2)*3 = .09/.34*3 ≈ 0.7941。
        assert!((out[1].place_prob - 0.4).abs() < 1e-9);
        assert!((out[1].show_prob - 0.09 / 0.34 * 3.0).abs() < 1e-9);
        // 単調性。
        for p in &out {
            assert!(p.win_prob <= p.place_prob + 1e-12 && p.place_prob <= p.show_prob + 1e-12);
        }
    }

    #[test]
    fn preserves_ranking_and_sharpens_spread() {
        // 単調変換なので place/show のランクは保存し、相対スプレッド（比）は広がる（シャープ化）。
        let probs = vec![
            prob(1, 0.05, 0.10, 0.15),
            prob(2, 0.04, 0.08, 0.12),
            prob(3, 0.03, 0.06, 0.10),
        ];
        let out = apply_placeshow_power(&probs, 2.0);
        // show のランク保存（入力 1>2>3）。
        assert!(out[0].show_prob > out[1].show_prob && out[1].show_prob > out[2].show_prob);
        // 比が広がる: out 比 > in 比（cap 未到達の小さい値で検証）。
        let in_ratio = probs[0].show_prob / probs[2].show_prob;
        let out_ratio = out[0].show_prob / out[2].show_prob;
        assert!(out_ratio > in_ratio, "out {out_ratio} > in {in_ratio}");
    }

    #[test]
    fn no_op_conditions() {
        let probs = vec![prob(1, 0.3, 0.4, 0.5), prob(2, 0.1, 0.2, 0.3)];
        // γ≈1.0 / ≤0 / 非有限 / 空 は no-op。
        for g in [1.0, 0.0, -1.0, f64::NAN, f64::INFINITY] {
            let out = apply_placeshow_power(&probs, g);
            for (a, b) in out.iter().zip(&probs) {
                assert_eq!(a.place_prob, b.place_prob);
                assert_eq!(a.show_prob, b.show_prob);
            }
        }
        assert!(apply_placeshow_power(&[], 2.0).is_empty());
    }
}
