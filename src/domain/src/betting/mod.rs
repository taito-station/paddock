use std::collections::HashMap;

use ordered_float::OrderedFloat;

use crate::horse_result::HorseNum;
use crate::odds::{OrderedPair, OrderedTriple, Pair, RaceOdds, Triple};
use crate::prediction::HorseProbability;

const MIN_DENOMINATOR: f64 = 1e-6;

#[derive(Debug, Clone)]
pub struct BettingConfig {
    pub ev_threshold: f64,
    pub trifecta_ev_threshold: f64,
    pub kelly_cap: f64,
}

impl Default for BettingConfig {
    fn default() -> Self {
        Self {
            ev_threshold: 1.0,
            trifecta_ev_threshold: 2.0,
            kelly_cap: 0.25,
        }
    }
}

#[derive(Debug, Clone)]
pub enum BetCombination {
    Win(HorseNum),
    Place(HorseNum),
    Quinella(Pair),
    Exacta(OrderedPair),
    Trio(Triple),
    Trifecta(OrderedTriple),
}

#[derive(Debug, Clone)]
pub struct BettingRecommendation {
    pub combination: BetCombination,
    pub probability: f64,
    pub odds: f64,
    pub ev: f64,
    pub kelly_fraction: f64,
}

/// Returns EV-positive bet recommendations sorted by bet-type priority then EV descending.
///
/// Priority: Quinella(0) > Exacta(1) > Trio(2) > Win(3) > Place(4) > Trifecta(5).
/// Trifecta candidates require `ev > config.trifecta_ev_threshold`; all others
/// require `ev > config.ev_threshold`.
pub fn select_bets(
    probabilities: &[HorseProbability],
    race_odds: &RaceOdds,
    config: &BettingConfig,
) -> Vec<BettingRecommendation> {
    let win_map: HashMap<HorseNum, f64> = probabilities
        .iter()
        .map(|p| (p.horse_num, p.win_prob))
        .collect();
    let show_map: HashMap<HorseNum, f64> = probabilities
        .iter()
        .map(|p| (p.horse_num, p.show_prob))
        .collect();

    let mut recs: Vec<BettingRecommendation> = Vec::new();

    for (&horse, &ov) in &race_odds.win {
        if let Some(&p) = win_map.get(&horse) {
            let o = ov.value();
            push_if_positive(&mut recs, BetCombination::Win(horse), p, o, config.ev_threshold, config.kelly_cap);
        }
    }

    for (&horse, place_ov) in &race_odds.place {
        if let Some(&p) = show_map.get(&horse) {
            let o = (place_ov.low.value() + place_ov.high.value()) / 2.0;
            push_if_positive(&mut recs, BetCombination::Place(horse), p, o, config.ev_threshold, config.kelly_cap);
        }
    }

    for (&pair, &ov) in &race_odds.quinella {
        let (a, b) = pair.as_tuple();
        if let (Some(&wa), Some(&wb)) = (win_map.get(&a), win_map.get(&b)) {
            let p = harville_quinella(wa, wb);
            let o = ov.value();
            push_if_positive(&mut recs, BetCombination::Quinella(pair), p, o, config.ev_threshold, config.kelly_cap);
        }
    }

    for (&pair, &ov) in &race_odds.exacta {
        let (a, b) = pair.as_tuple();
        if let (Some(&wa), Some(&wb)) = (win_map.get(&a), win_map.get(&b)) {
            let p = harville_exacta(wa, wb);
            let o = ov.value();
            push_if_positive(&mut recs, BetCombination::Exacta(pair), p, o, config.ev_threshold, config.kelly_cap);
        }
    }

    for (&triple, &ov) in &race_odds.trio {
        let (a, b, c) = triple.as_tuple();
        if let (Some(&wa), Some(&wb), Some(&wc)) = (win_map.get(&a), win_map.get(&b), win_map.get(&c)) {
            let p = harville_trio(wa, wb, wc);
            let o = ov.value();
            push_if_positive(&mut recs, BetCombination::Trio(triple), p, o, config.ev_threshold, config.kelly_cap);
        }
    }

    for (&triple, &ov) in &race_odds.trifecta {
        let (a, b, c) = triple.as_tuple();
        if let (Some(&wa), Some(&wb), Some(&wc)) = (win_map.get(&a), win_map.get(&b), win_map.get(&c)) {
            let p = harville_trifecta(wa, wb, wc);
            let o = ov.value();
            push_if_positive(&mut recs, BetCombination::Trifecta(triple), p, o, config.trifecta_ev_threshold, config.kelly_cap);
        }
    }

    recs.sort_by_key(|r| (priority(&r.combination), OrderedFloat(-r.ev)));
    recs
}

fn push_if_positive(
    recs: &mut Vec<BettingRecommendation>,
    combination: BetCombination,
    probability: f64,
    odds: f64,
    threshold: f64,
    kelly_cap: f64,
) {
    let ev = probability * odds;
    if ev > threshold {
        recs.push(BettingRecommendation {
            combination,
            probability,
            odds,
            ev,
            kelly_fraction: kelly_fraction(probability, odds, kelly_cap),
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
    }
}

/// P(a→b): Harville conditional probability that b finishes 2nd given a wins.
pub fn harville_exacta(win_a: f64, win_b: f64) -> f64 {
    let denom = (1.0 - win_a).max(MIN_DENOMINATOR);
    win_a * win_b / denom
}

/// P(quinella {a,b}) = P(a→b) + P(b→a).
pub fn harville_quinella(win_a: f64, win_b: f64) -> f64 {
    harville_exacta(win_a, win_b) + harville_exacta(win_b, win_a)
}

/// P(trifecta a→b→c): Harville sequential conditional probability.
pub fn harville_trifecta(win_a: f64, win_b: f64, win_c: f64) -> f64 {
    let denom_a = (1.0 - win_a).max(MIN_DENOMINATOR);
    let denom_ab = (1.0 - win_a - win_b).max(MIN_DENOMINATOR);
    win_a * win_b / denom_a * win_c / denom_ab
}

/// P(trio {a,b,c}) = sum of all 6 ordered permutations as trifecta probabilities.
pub fn harville_trio(win_a: f64, win_b: f64, win_c: f64) -> f64 {
    harville_trifecta(win_a, win_b, win_c)
        + harville_trifecta(win_a, win_c, win_b)
        + harville_trifecta(win_b, win_a, win_c)
        + harville_trifecta(win_b, win_c, win_a)
        + harville_trifecta(win_c, win_a, win_b)
        + harville_trifecta(win_c, win_b, win_a)
}

/// Kelly fraction with cap: f = (p*b - q) / b, clamped to [0, kelly_cap].
///
/// `gross_odds` is the JRA payout multiplier (e.g. 3.5 means ¥350 back on ¥100).
/// Net odds b = gross_odds - 1.
pub fn kelly_fraction(p: f64, gross_odds: f64, kelly_cap: f64) -> f64 {
    let b = gross_odds - 1.0;
    let q = 1.0 - p;
    let f = (p * b - q) / b;
    f.max(0.0).min(kelly_cap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horse_result::HorseNum;
    use crate::odds::{OddsValue, PlaceOdds, RaceOdds};
    use crate::prediction::HorseProbability;
    use crate::race::RaceId;

    fn make_race_id() -> RaceId {
        RaceId::try_from("202506040101".to_string()).unwrap()
    }

    fn horse(n: u32) -> HorseNum {
        HorseNum::try_from(n).unwrap()
    }

    fn odds(v: f64) -> OddsValue {
        OddsValue::try_from(v).unwrap()
    }

    fn place_odds(lo: f64, hi: f64) -> PlaceOdds {
        PlaceOdds::try_from((odds(lo), odds(hi))).unwrap()
    }

    fn prob(horse_num: u32, win: f64, show: f64) -> HorseProbability {
        use crate::horse_result::HorseName;
        HorseProbability {
            horse_num: horse(horse_num),
            horse_name: HorseName::try_from(format!("ウマ{horse_num}")).unwrap(),
            win_prob: win,
            place_prob: win + show / 2.0,
            show_prob: show,
        }
    }

    #[test]
    fn empty_probabilities_returns_empty() {
        let race_odds = RaceOdds::empty(make_race_id());
        let result = select_bets(&[], &race_odds, &BettingConfig::default());
        assert!(result.is_empty());
    }

    #[test]
    fn empty_odds_returns_empty() {
        let probs = vec![prob(1, 0.5, 0.7), prob(2, 0.3, 0.5)];
        let race_odds = RaceOdds::empty(make_race_id());
        let result = select_bets(&probs, &race_odds, &BettingConfig::default());
        assert!(result.is_empty());
    }

    #[test]
    fn win_bet_above_threshold_is_included() {
        let probs = vec![prob(1, 0.4, 0.6)];
        let mut race_odds = RaceOdds::empty(make_race_id());
        race_odds.win.insert(horse(1), odds(3.5));

        let result = select_bets(&probs, &race_odds, &BettingConfig::default());
        assert_eq!(result.len(), 1);
        let r = &result[0];
        assert!((r.ev - 0.4 * 3.5).abs() < 1e-10);
        assert!(r.ev > 1.0);
    }

    #[test]
    fn win_bet_below_threshold_is_excluded() {
        // p=0.2, odds=4.0, ev=0.8 < 1.0
        let probs = vec![prob(1, 0.2, 0.4)];
        let mut race_odds = RaceOdds::empty(make_race_id());
        race_odds.win.insert(horse(1), odds(4.0));

        let result = select_bets(&probs, &race_odds, &BettingConfig::default());
        assert!(result.is_empty());
    }

    #[test]
    fn trifecta_uses_higher_threshold() {
        // EV = 0.01 * 150.0 = 1.5 — above ev_threshold(1.0) but below trifecta_ev_threshold(2.0)
        let probs = vec![
            prob(1, 0.4, 0.6),
            prob(2, 0.35, 0.55),
            prob(3, 0.2, 0.4),
        ];
        let (a, b, c) = (horse(1), horse(2), horse(3));
        let triple = OrderedTriple::try_from((a, b, c)).unwrap();
        let mut race_odds = RaceOdds::empty(make_race_id());
        // Set odds high enough for EV > 1.0 but < 2.0
        // harville_trifecta(0.4, 0.35, 0.2) ≈ 0.4 * 0.35/0.6 * 0.2/0.25 ≈ 0.187
        // To get EV = 1.5: odds ≈ 1.5 / 0.187 ≈ 8.0
        race_odds.trifecta.insert(triple, odds(8.0));

        let result = select_bets(&probs, &race_odds, &BettingConfig::default());
        // EV should be around 1.5 which is < trifecta_ev_threshold(2.0) → excluded
        assert!(result.is_empty(), "trifecta with EV < 2.0 should be excluded");
    }

    #[test]
    fn trifecta_above_trifecta_threshold_is_included() {
        let probs = vec![
            prob(1, 0.4, 0.6),
            prob(2, 0.35, 0.55),
            prob(3, 0.2, 0.4),
        ];
        let (a, b, c) = (horse(1), horse(2), horse(3));
        let triple = OrderedTriple::try_from((a, b, c)).unwrap();
        let mut race_odds = RaceOdds::empty(make_race_id());
        // Use odds large enough to exceed trifecta_ev_threshold(2.0)
        race_odds.trifecta.insert(triple, odds(20.0));

        let result = select_bets(&probs, &race_odds, &BettingConfig::default());
        assert!(!result.is_empty());
        let r = &result[0];
        assert!(r.ev > 2.0);
    }

    #[test]
    fn kelly_fraction_is_capped() {
        // Very high win probability → kelly would exceed cap without clamping
        let probs = vec![prob(1, 0.9, 0.95)];
        let mut race_odds = RaceOdds::empty(make_race_id());
        race_odds.win.insert(horse(1), odds(2.0));

        let config = BettingConfig { ev_threshold: 1.0, trifecta_ev_threshold: 2.0, kelly_cap: 0.25 };
        let result = select_bets(&probs, &race_odds, &config);
        assert!(!result.is_empty());
        assert!(result[0].kelly_fraction <= 0.25);
    }

    #[test]
    fn quinella_priority_before_win() {
        // Both quinella and win should appear; quinella (priority 0) before win (priority 3)
        let probs = vec![prob(1, 0.5, 0.7), prob(2, 0.3, 0.55)];
        let pair = Pair::try_from((horse(1), horse(2))).unwrap();
        let mut race_odds = RaceOdds::empty(make_race_id());
        race_odds.win.insert(horse(1), odds(2.5));  // EV = 0.5*2.5 = 1.25
        race_odds.quinella.insert(pair, odds(5.0)); // EV = harville_quinella(0.5,0.3)*5.0

        let result = select_bets(&probs, &race_odds, &BettingConfig::default());
        assert!(result.len() >= 2);
        assert!(matches!(result[0].combination, BetCombination::Quinella(_)));
    }

    #[test]
    fn place_uses_midpoint_odds() {
        let probs = vec![prob(1, 0.4, 0.6)];
        let mut race_odds = RaceOdds::empty(make_race_id());
        race_odds.place.insert(horse(1), place_odds(2.0, 4.0));

        let result = select_bets(&probs, &race_odds, &BettingConfig::default());
        // midpoint = (2.0+4.0)/2 = 3.0, EV = 0.6*3.0 = 1.8 > 1.0
        assert_eq!(result.len(), 1);
        assert!((result[0].odds - 3.0).abs() < 1e-10);
        assert!((result[0].ev - 1.8).abs() < 1e-10);
    }

    #[test]
    fn harville_exacta_formula() {
        // P(1→2) = win[1] * win[2] / (1 - win[1])
        let result = harville_exacta(0.4, 0.3);
        let expected = 0.4 * 0.3 / (1.0 - 0.4);
        assert!((result - expected).abs() < 1e-10);
    }

    #[test]
    fn harville_quinella_is_sum_of_two_exactas() {
        let wa = 0.4;
        let wb = 0.3;
        let q = harville_quinella(wa, wb);
        let expected = harville_exacta(wa, wb) + harville_exacta(wb, wa);
        assert!((q - expected).abs() < 1e-10);
    }

    #[test]
    fn harville_trio_is_sum_of_six_trifectas() {
        let (wa, wb, wc) = (0.4, 0.3, 0.2);
        let trio = harville_trio(wa, wb, wc);
        let expected = harville_trifecta(wa, wb, wc)
            + harville_trifecta(wa, wc, wb)
            + harville_trifecta(wb, wa, wc)
            + harville_trifecta(wb, wc, wa)
            + harville_trifecta(wc, wa, wb)
            + harville_trifecta(wc, wb, wa);
        assert!((trio - expected).abs() < 1e-10);
    }

    #[test]
    fn kelly_fraction_basic() {
        // p=0.4, odds=3.5: b=2.5, q=0.6, f=(0.4*2.5-0.6)/2.5=0.16
        let kf = kelly_fraction(0.4, 3.5, 0.25);
        assert!((kf - 0.16).abs() < 1e-10);
    }

    #[test]
    fn kelly_fraction_negative_clamped_to_zero() {
        // p=0.1, odds=2.0: b=1.0, q=0.9, f=(0.1-0.9)/1.0=-0.8 → clamped to 0
        let kf = kelly_fraction(0.1, 2.0, 0.25);
        assert_eq!(kf, 0.0);
    }

    #[test]
    fn kelly_fraction_respects_cap() {
        // Large win prob produces f > cap
        let kf = kelly_fraction(0.95, 2.0, 0.25);
        assert_eq!(kf, 0.25);
    }
}
