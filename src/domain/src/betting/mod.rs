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
    /// Gross payout multiplier. `ev = probability * odds`.
    /// - 単勝/馬連/馬単/三連複/三連単: JRA が公表するオッズそのまま
    /// - 複勝（`BetCombination::Place`）: オッズ幅 `(low..high)` の中央値を代入
    pub odds: f64,
    pub ev: f64,
    pub kelly_fraction: f64,
}

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
            push_if_positive(&mut recs, BetCombination::Win(horse), hp.win_prob, o, config.ev_threshold, config);
        }
    }

    for (&horse, place_ov) in &race_odds.place {
        if let Some(hp) = prob_map.get(&horse) {
            // JRA 複勝は「3 着以内入線」に相当するため show_prob（3 着以内確率）を使用する。
            // place_prob は「2 着以内確率」（連対率）であり、複勝計算では使わない。
            // 未確定幅 (low..high) の中央値を期待値計算の代表値とする。
            let o = (place_ov.low.value() + place_ov.high.value()) / 2.0;
            push_if_positive(&mut recs, BetCombination::Place(horse), hp.show_prob, o, config.ev_threshold, config);
        }
    }

    for (&pair, &ov) in &race_odds.quinella {
        let (a, b) = pair.as_tuple();
        if let (Some(ha), Some(hb)) = (prob_map.get(&a), prob_map.get(&b)) {
            let p = harville_quinella(ha.win_prob, hb.win_prob);
            let o = ov.value();
            push_if_positive(&mut recs, BetCombination::Quinella(pair), p, o, config.ev_threshold, config);
        }
    }

    for (&pair, &ov) in &race_odds.exacta {
        let (a, b) = pair.as_tuple();
        if let (Some(ha), Some(hb)) = (prob_map.get(&a), prob_map.get(&b)) {
            let p = harville_exacta(ha.win_prob, hb.win_prob);
            let o = ov.value();
            push_if_positive(&mut recs, BetCombination::Exacta(pair), p, o, config.ev_threshold, config);
        }
    }

    for (&triple, &ov) in &race_odds.trio {
        let (a, b, c) = triple.as_tuple();
        if let (Some(ha), Some(hb), Some(hc)) = (prob_map.get(&a), prob_map.get(&b), prob_map.get(&c)) {
            let p = harville_trio(ha.win_prob, hb.win_prob, hc.win_prob);
            let o = ov.value();
            push_if_positive(&mut recs, BetCombination::Trio(triple), p, o, config.ev_threshold, config);
        }
    }

    for (&triple, &ov) in &race_odds.trifecta {
        let (a, b, c) = triple.as_tuple();
        if let (Some(ha), Some(hb), Some(hc)) = (prob_map.get(&a), prob_map.get(&b), prob_map.get(&c)) {
            let p = harville_trifecta(ha.win_prob, hb.win_prob, hc.win_prob);
            let o = ov.value();
            push_if_positive(&mut recs, BetCombination::Trifecta(triple), p, o, config.trifecta_ev_threshold, config);
        }
    }

    recs.sort_by_key(|r| (priority(&r.combination), OrderedFloat(-r.ev), combination_ord_key(&r.combination)));
    recs
}

fn combination_ord_key(c: &BetCombination) -> (u32, u32, u32) {
    match c {
        BetCombination::Win(h) | BetCombination::Place(h) => (h.value(), 0, 0),
        BetCombination::Quinella(p) => { let (a, b) = p.as_tuple(); (a.value(), b.value(), 0) }
        BetCombination::Exacta(p) => { let (a, b) = p.as_tuple(); (a.value(), b.value(), 0) }
        BetCombination::Trio(t) => { let (a, b, c) = t.as_tuple(); (a.value(), b.value(), c.value()) }
        BetCombination::Trifecta(t) => { let (a, b, c) = t.as_tuple(); (a.value(), b.value(), c.value()) }
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
    }
}

/// P(a→b): Harville conditional probability that b finishes 2nd given a wins.
///
/// Returns `0.0` when `win_a + win_b >= 1.0`: the first horse leaving no probability
/// mass for others to finish behind the second horse would produce an invalid result.
pub fn harville_exacta(win_a: f64, win_b: f64) -> f64 {
    if win_a + win_b >= 1.0 {
        return 0.0;
    }
    let denom = (1.0 - win_a).max(MIN_DENOMINATOR);
    win_a * win_b / denom
}

/// P(quinella {a,b}) = P(a→b) + P(b→a).
pub fn harville_quinella(win_a: f64, win_b: f64) -> f64 {
    harville_exacta(win_a, win_b) + harville_exacta(win_b, win_a)
}

/// P(trifecta a→b→c): Harville sequential conditional probability.
///
/// Precondition: `win_a + win_b < 1.0`. Returns `0.0` when this is violated
/// to avoid a negative denominator being clamped to MIN_DENOMINATOR, which
/// would produce an unrealistically large probability.
pub fn harville_trifecta(win_a: f64, win_b: f64, win_c: f64) -> f64 {
    if win_a + win_b >= 1.0 {
        return 0.0;
    }
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
/// Net odds b = gross_odds - 1.0 (gross → net 変換). EV = p * gross_odds; EV > 1.0 が期待値プラス。
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
            // place_prob（2着以内確率）は select_bets で使わない。
            // 0.0 にしておくことで誤って参照された場合に EV が0になりすぐ気づける。
            place_prob: 0.0,
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
        // harville_trifecta(0.4, 0.35, 0.2) ≈ 0.4 * 0.35/0.6 * 0.2/0.25 ≈ 0.187
        // EV ≈ 0.187 * 8.0 ≈ 1.5 → above ev_threshold(1.0) but below trifecta_ev_threshold(2.0)
        let probs = vec![
            prob(1, 0.4, 0.6),
            prob(2, 0.35, 0.55),
            prob(3, 0.2, 0.4),
        ];
        let (a, b, c) = (horse(1), horse(2), horse(3));
        let triple = OrderedTriple::try_from((a, b, c)).unwrap();
        let mut race_odds = RaceOdds::empty(make_race_id());
        race_odds.trifecta.insert(triple, odds(8.0));

        let result = select_bets(&probs, &race_odds, &BettingConfig::default());
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
        race_odds.trifecta.insert(triple, odds(20.0));

        let result = select_bets(&probs, &race_odds, &BettingConfig::default());
        assert!(!result.is_empty());
        let r = &result[0];
        assert!(r.ev > 2.0);
    }

    #[test]
    fn kelly_fraction_is_capped() {
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
        let probs = vec![prob(1, 0.5, 0.7), prob(2, 0.3, 0.55)];
        let pair = Pair::try_from((horse(1), horse(2))).unwrap();
        let mut race_odds = RaceOdds::empty(make_race_id());
        race_odds.win.insert(horse(1), odds(2.5));  // EV = 0.5*2.5 = 1.25
        race_odds.quinella.insert(pair, odds(5.0));

        let result = select_bets(&probs, &race_odds, &BettingConfig::default());
        assert!(result.iter().any(|r| matches!(r.combination, BetCombination::Quinella(_))));
        assert!(result.iter().any(|r| matches!(r.combination, BetCombination::Win(_))));
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
        let result = harville_exacta(0.4, 0.3);
        let expected = 0.4 * 0.3 / (1.0 - 0.4);
        assert!((result - expected).abs() < 1e-10);
    }

    #[test]
    fn harville_exacta_returns_zero_when_sum_exhausts_probability() {
        // win_a + win_b >= 1.0 → guard returns 0.0
        assert_eq!(harville_exacta(1.0, 0.0), 0.0);
        assert_eq!(harville_exacta(0.6, 0.5), 0.0);
        assert_eq!(harville_exacta(0.5, 0.5), 0.0);
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
    fn harville_trifecta_returns_zero_when_first_two_exhaust_probability() {
        // win_a + win_b >= 1.0 → guard returns 0.0 instead of clamped huge value
        assert_eq!(harville_trifecta(0.6, 0.5, 0.1), 0.0);
        assert_eq!(harville_trifecta(1.0, 0.0, 0.0), 0.0);
    }

    #[test]
    fn harville_trio_with_near_unit_sum_returns_finite_value() {
        // wa+wb+wc = 0.95; some permutations will trigger trifecta guard
        // (e.g. wb+wa = 0.5+0.4 = 0.9 < 1.0 is ok; but wc=0.05 combos are fine)
        let trio = harville_trio(0.5, 0.4, 0.05);
        assert!(trio >= 0.0);
        assert!(trio <= 1.0, "trio probability should not exceed 1.0, got {trio}");
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
        let kf = kelly_fraction(0.95, 2.0, 0.25);
        assert_eq!(kf, 0.25);
    }
}
