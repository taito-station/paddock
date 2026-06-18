//! EV プラスの買い目選定（Harville/Kelly を用いた採点・並べ替え・curation）。

use std::collections::HashMap;

use ordered_float::OrderedFloat;

use super::harville::{harville_exacta, harville_quinella, harville_trifecta, harville_trio};
use super::kelly::kelly_fraction;
use super::model::{BetCombination, BettingConfig, BettingRecommendation};
use crate::horse_result::HorseNum;
use crate::odds::RaceOdds;
use crate::prediction::HorseProbability;

/// Returns EV-positive bet recommendations sorted by bet-type priority then EV descending.
///
/// Priority (sort key): Quinella(0) > Exacta(1) > Trio(2) > Win(3) > Place(4) > Trifecta(5).
/// Smaller sort key = earlier in the result. Trifecta requires `ev > trifecta_ev_threshold`;
/// all other bet types require `ev > ev_threshold` (strict greater-than; ev = threshold is excluded).
/// When two recommendations share the same priority and EV, they are ordered by horse numbers
/// for deterministic output.
///
/// Note: `Place` (複勝) uses `HorseProbability::show_prob` (3着以内確率) as the probability estimate.
/// `place_prob` (2着以内確率) is not used for any bet type in this function.
pub fn select_bets(
    probabilities: &[HorseProbability],
    race_odds: &RaceOdds,
    config: &BettingConfig,
) -> Vec<BettingRecommendation> {
    let prob_map: HashMap<HorseNum, &HorseProbability> =
        probabilities.iter().map(|p| (p.horse_num, p)).collect();

    let mut recs: Vec<BettingRecommendation> = Vec::new();

    for (&horse, &ov) in &race_odds.win {
        if let Some(hp) = prob_map.get(&horse) {
            let o = ov.value();
            push_if_positive(
                &mut recs,
                BetCombination::Win(horse),
                hp.win_prob,
                o,
                config.ev_threshold,
                config,
            );
        }
    }

    for (&horse, place_ov) in &race_odds.place {
        if let Some(hp) = prob_map.get(&horse) {
            // JRA 複勝は「3 着以内入線」に相当するため show_prob（3 着以内確率）を使用する。
            // place_prob は「2 着以内確率」（連対率）であり、複勝計算では使わない。
            // 未確定幅 (low..high) の中央値を期待値計算の代表値とする。
            let o = (place_ov.low.value() + place_ov.high.value()) / 2.0;
            push_if_positive(
                &mut recs,
                BetCombination::Place(horse),
                hp.show_prob,
                o,
                config.ev_threshold,
                config,
            );
        }
    }

    for (&pair, &ov) in &race_odds.quinella {
        let (a, b) = pair.as_tuple();
        if let (Some(ha), Some(hb)) = (prob_map.get(&a), prob_map.get(&b)) {
            let p = harville_quinella(ha.win_prob, hb.win_prob);
            let o = ov.value();
            push_if_positive(
                &mut recs,
                BetCombination::Quinella(pair),
                p,
                o,
                config.ev_threshold,
                config,
            );
        }
    }

    for (&pair, &ov) in &race_odds.exacta {
        let (a, b) = pair.as_tuple();
        if let (Some(ha), Some(hb)) = (prob_map.get(&a), prob_map.get(&b)) {
            let p = harville_exacta(ha.win_prob, hb.win_prob);
            let o = ov.value();
            push_if_positive(
                &mut recs,
                BetCombination::Exacta(pair),
                p,
                o,
                config.ev_threshold,
                config,
            );
        }
    }

    for (&triple, &ov) in &race_odds.trio {
        let (a, b, c) = triple.as_tuple();
        if let (Some(ha), Some(hb), Some(hc)) =
            (prob_map.get(&a), prob_map.get(&b), prob_map.get(&c))
        {
            let p = harville_trio(ha.win_prob, hb.win_prob, hc.win_prob);
            let o = ov.value();
            push_if_positive(
                &mut recs,
                BetCombination::Trio(triple),
                p,
                o,
                config.ev_threshold,
                config,
            );
        }
    }

    for (&triple, &ov) in &race_odds.trifecta {
        let (a, b, c) = triple.as_tuple();
        if let (Some(ha), Some(hb), Some(hc)) =
            (prob_map.get(&a), prob_map.get(&b), prob_map.get(&c))
        {
            let p = harville_trifecta(ha.win_prob, hb.win_prob, hc.win_prob);
            let o = ov.value();
            push_if_positive(
                &mut recs,
                BetCombination::Trifecta(triple),
                p,
                o,
                config.trifecta_ev_threshold,
                config,
            );
        }
    }

    recs.sort_by_key(|r| {
        (
            priority(&r.combination),
            OrderedFloat(-r.ev),
            combination_ord_key(&r.combination),
        )
    });

    // curation（#121）: 全正EVダンプを止める。EV 閾値通過後の買い目から、
    // (1) Kelly が薄い（過信に支えられた一点¥1〜3 級の）買い目を落とし、
    // (2) 券種ごとに EV 上位 N 点へ絞る。recs は priority→EV 降順済みで、券種は
    //     priority と 1:1 のため、type 別カウンタで残せば EV 上位 N が保たれる。
    recs.retain(|r| r.kelly_fraction > config.min_kelly);
    if let Some(n) = config.max_bets_per_type {
        let mut per_type: HashMap<&'static str, usize> = HashMap::new();
        recs.retain(|r| {
            let c = per_type.entry(r.combination.type_label()).or_insert(0);
            if *c < n {
                *c += 1;
                true
            } else {
                false
            }
        });
    }
    recs
}

fn combination_ord_key(c: &BetCombination) -> (u32, u32, u32) {
    match c {
        BetCombination::Win(h) | BetCombination::Place(h) => (h.value(), 0, 0),
        BetCombination::Quinella(p) | BetCombination::Wide(p) => {
            let (a, b) = p.as_tuple();
            (a.value(), b.value(), 0)
        }
        BetCombination::Exacta(p) => {
            let (a, b) = p.as_tuple();
            (a.value(), b.value(), 0)
        }
        BetCombination::Trio(t) => {
            let (a, b, c) = t.as_tuple();
            (a.value(), b.value(), c.value())
        }
        BetCombination::Trifecta(t) => {
            let (a, b, c) = t.as_tuple();
            (a.value(), b.value(), c.value())
        }
    }
}

fn push_if_positive(
    recs: &mut Vec<BettingRecommendation>,
    combination: BetCombination,
    probability: f64,
    odds: f64,
    ev_threshold: f64,
    config: &BettingConfig,
) {
    let ev = probability * odds;
    if ev > ev_threshold {
        recs.push(BettingRecommendation {
            combination,
            probability,
            odds,
            ev,
            kelly_fraction: kelly_fraction(probability, odds, config.kelly_cap),
        });
    }
}

fn priority(c: &BetCombination) -> u8 {
    match c {
        BetCombination::Quinella(_) => 0,
        BetCombination::Exacta(_) => 1,
        BetCombination::Trio(_) => 2,
        BetCombination::Win(_) => 3,
        BetCombination::Place(_) => 4,
        BetCombination::Trifecta(_) => 5,
        // Wide は select_bets では生成しない（収支シミュレータ専用）。網羅性のため末尾に置く。
        BetCombination::Wide(_) => 6,
    }
}
