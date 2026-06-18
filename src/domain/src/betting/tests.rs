use super::harville::{harville_exacta, harville_quinella, harville_trifecta, harville_trio};
use super::hit::bet_hit;
use super::kelly::kelly_fraction;
use super::model::{BetCombination, BettingConfig, Podium};
use super::select::select_bets;
use crate::horse_result::HorseNum;
use crate::odds::{OddsValue, OrderedPair, OrderedTriple, Pair, PlaceOdds, RaceOdds, Triple};
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
    let probs = vec![prob(1, 0.4, 0.6), prob(2, 0.35, 0.55), prob(3, 0.2, 0.4)];
    let (a, b, c) = (horse(1), horse(2), horse(3));
    let triple = OrderedTriple::try_from((a, b, c)).unwrap();
    let mut race_odds = RaceOdds::empty(make_race_id());
    race_odds.trifecta.insert(triple, odds(8.0));

    let result = select_bets(&probs, &race_odds, &BettingConfig::default());
    assert!(
        result.is_empty(),
        "trifecta with EV < 2.0 should be excluded"
    );
}

#[test]
fn trifecta_above_trifecta_threshold_is_included() {
    let probs = vec![prob(1, 0.4, 0.6), prob(2, 0.35, 0.55), prob(3, 0.2, 0.4)];
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

    let config = BettingConfig {
        ev_threshold: 1.0,
        trifecta_ev_threshold: 2.0,
        kelly_cap: 0.25,
        min_kelly: 0.0,
        max_bets_per_type: None,
    };
    let result = select_bets(&probs, &race_odds, &config);
    assert!(!result.is_empty());
    assert!(result[0].kelly_fraction <= 0.25);
}

#[test]
fn quinella_priority_before_win() {
    let probs = vec![prob(1, 0.5, 0.7), prob(2, 0.3, 0.55)];
    let pair = Pair::try_from((horse(1), horse(2))).unwrap();
    let mut race_odds = RaceOdds::empty(make_race_id());
    race_odds.win.insert(horse(1), odds(2.5)); // EV = 0.5*2.5 = 1.25
    race_odds.quinella.insert(pair, odds(5.0));

    let result = select_bets(&probs, &race_odds, &BettingConfig::default());
    assert!(
        result
            .iter()
            .any(|r| matches!(r.combination, BetCombination::Quinella(_)))
    );
    assert!(
        result
            .iter()
            .any(|r| matches!(r.combination, BetCombination::Win(_)))
    );
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
fn min_kelly_filters_thin_positive_ev_bets() {
    // EV はわずかに正だが Kelly が薄い買い目（全正EVダンプの主因, #121）。
    // p=0.05, odds=21 → EV=1.05>1.0 だが Kelly=(0.05*20-0.95)/20=0.0025。
    let probs = vec![prob(1, 0.05, 0.1)];
    let mut race_odds = RaceOdds::empty(make_race_id());
    race_odds.win.insert(horse(1), odds(21.0));

    // uncurated（min_kelly=0）なら EV>1 で採用される。
    let uncurated = BettingConfig {
        ev_threshold: 1.0,
        trifecta_ev_threshold: 2.0,
        kelly_cap: 0.25,
        min_kelly: 0.0,
        max_bets_per_type: None,
    };
    assert_eq!(select_bets(&probs, &race_odds, &uncurated).len(), 1);
    // 既定（min_kelly=0.01）では Kelly 薄として除外される。
    assert!(select_bets(&probs, &race_odds, &BettingConfig::default()).is_empty());
}

#[test]
fn max_bets_per_type_caps_to_top_n_by_ev() {
    // 同一券種（単勝）で +EV かつ Kelly 十分な 4 頭を用意し、券種上限 2 に絞る。
    let probs = vec![
        prob(1, 0.5, 0.7),
        prob(2, 0.45, 0.65),
        prob(3, 0.4, 0.6),
        prob(4, 0.35, 0.55),
    ];
    let mut race_odds = RaceOdds::empty(make_race_id());
    for h in 1..=4 {
        race_odds.win.insert(horse(h), odds(3.0)); // EV = win_prob * 3.0
    }
    let config = BettingConfig {
        ev_threshold: 1.0,
        trifecta_ev_threshold: 2.0,
        kelly_cap: 0.25,
        min_kelly: 0.0,
        max_bets_per_type: Some(2),
    };
    let result = select_bets(&probs, &race_odds, &config);
    let wins: Vec<_> = result
        .iter()
        .filter(|r| matches!(r.combination, BetCombination::Win(_)))
        .collect();
    assert_eq!(wins.len(), 2, "券種上限 2 に絞られる");
    // EV 上位（馬1=1.5, 馬2=1.35）が残り、下位（馬3=1.2, 馬4=1.05）は落ちる。
    assert!(wins.iter().all(|r| r.ev >= 1.35 - 1e-9));
}

#[test]
fn min_kelly_and_max_bets_per_type_compose() {
    // 両 curation レバーの合成。単勝 5 頭: 上位 4 頭は Kelly 十分、5 頭目は EV>1 だが Kelly 薄。
    // min_kelly が先に 5 頭目を落とし、その後 max_bets_per_type=2 で EV 上位 2 点に絞られる。
    let probs = vec![
        prob(1, 0.5, 0.7),
        prob(2, 0.45, 0.65),
        prob(3, 0.4, 0.6),
        prob(4, 0.35, 0.55),
        prob(5, 0.05, 0.1), // EV=1.05 だが Kelly=0.0025（薄い）
    ];
    let mut race_odds = RaceOdds::empty(make_race_id());
    for h in 1..=4 {
        race_odds.win.insert(horse(h), odds(3.0)); // EV = win_prob * 3.0
    }
    race_odds.win.insert(horse(5), odds(21.0)); // EV=1.05

    let config = BettingConfig {
        ev_threshold: 1.0,
        trifecta_ev_threshold: 2.0,
        kelly_cap: 0.25,
        min_kelly: 0.01,
        max_bets_per_type: Some(2),
    };
    let result = select_bets(&probs, &race_odds, &config);
    assert_eq!(result.len(), 2, "min_kelly 除外後に券種上限 2 で絞られる");
    // 残るのは EV 上位の馬1(1.5)・馬2(1.35)。Kelly 薄の馬5 は min_kelly で先に脱落。
    assert!(result.iter().all(|r| r.ev >= 1.35 - 1e-9));
    assert!(
        result
            .iter()
            .all(|r| !matches!(&r.combination, BetCombination::Win(h) if h.value() == 5)),
        "Kelly 薄の馬5 は除外される"
    );
}

#[test]
fn bet_hit_judges_each_bet_type() {
    // 確定: 1着=3, 2着=1, 3着=5。8 頭立て（複勝/ワイドは 3 着以内まで払戻圏）。
    let podium = Podium {
        first: Some(horse(3)),
        second: Some(horse(1)),
        third: Some(horse(5)),
        field_size: 8,
    };
    assert!(bet_hit(&BetCombination::Win(horse(3)), &podium));
    assert!(!bet_hit(&BetCombination::Win(horse(1)), &podium));
    assert!(bet_hit(&BetCombination::Place(horse(5)), &podium));
    assert!(!bet_hit(&BetCombination::Place(horse(2)), &podium));
    let q = |a, b| BetCombination::Quinella(Pair::try_from((horse(a), horse(b))).unwrap());
    assert!(bet_hit(&q(1, 3), &podium)); // {1,2着}={3,1}
    assert!(!bet_hit(&q(3, 5), &podium));
    let ex =
        |a, b| BetCombination::Exacta(OrderedPair::try_from((horse(a), horse(b))).unwrap());
    assert!(bet_hit(&ex(3, 1), &podium)); // 1→2着=3→1
    assert!(!bet_hit(&ex(1, 3), &podium));
    let wd = |a, b| BetCombination::Wide(Pair::try_from((horse(a), horse(b))).unwrap());
    assert!(bet_hit(&wd(1, 5), &podium)); // 両馬3着以内
    assert!(!bet_hit(&wd(1, 2), &podium));
    let tr = |a, b, c| {
        BetCombination::Trio(Triple::try_from((horse(a), horse(b), horse(c))).unwrap())
    };
    assert!(bet_hit(&tr(5, 3, 1), &podium)); // {1,2,3着}無順
    assert!(!bet_hit(&tr(1, 2, 3), &podium));
    let tf = |a, b, c| {
        BetCombination::Trifecta(
            OrderedTriple::try_from((horse(a), horse(b), horse(c))).unwrap(),
        )
    };
    assert!(bet_hit(&tf(3, 1, 5), &podium)); // 1→2→3着
    assert!(!bet_hit(&tf(3, 5, 1), &podium));
}

#[test]
fn bet_hit_false_when_podium_incomplete() {
    // 1 着しか確定していない（同着・着順欠落など）。
    let podium = Podium {
        first: Some(horse(3)),
        second: None,
        third: None,
        field_size: 8,
    };
    assert!(bet_hit(&BetCombination::Win(horse(3)), &podium)); // 単勝は 1 着のみで判定可
    let q = BetCombination::Quinella(Pair::try_from((horse(3), horse(1))).unwrap());
    assert!(!bet_hit(&q, &podium)); // 2 着未確定 → 非的中
    let tf = BetCombination::Trifecta(
        OrderedTriple::try_from((horse(3), horse(1), horse(5))).unwrap(),
    );
    assert!(!bet_hit(&tf, &podium));
}

#[test]
fn bet_hit_place_and_wide_depend_on_field_size() {
    // 確定: 1着=3, 2着=1, 3着=5。複勝/ワイドの払戻圏は頭数依存（JRA: 8頭以上=3着, 7頭以下=2着）。
    let make = |field_size| Podium {
        first: Some(horse(3)),
        second: Some(horse(1)),
        third: Some(horse(5)),
        field_size,
    };
    // 7 頭立て: 3 着(=5)は払戻圏外。複勝 5 は不的中、ワイド{1,5}も 5 が圏外で不的中。
    let small = make(7);
    assert!(bet_hit(&BetCombination::Place(horse(1)), &small)); // 2 着は払戻圏
    assert!(!bet_hit(&BetCombination::Place(horse(5)), &small)); // 3 着は圏外
    let wd = |a, b| BetCombination::Wide(Pair::try_from((horse(a), horse(b))).unwrap());
    assert!(!bet_hit(&wd(1, 5), &small)); // 5 が圏外
    assert!(bet_hit(&wd(3, 1), &small)); // 1着・2着で両方圏内

    // 8 頭立て: 3 着(=5)まで払戻圏。複勝 5・ワイド{1,5}とも的中。
    let large = make(8);
    assert!(bet_hit(&BetCombination::Place(horse(5)), &large));
    assert!(bet_hit(&wd(1, 5), &large));
}

#[test]
fn harville_exacta_formula() {
    let result = harville_exacta(0.4, 0.3);
    let expected = 0.4 * 0.3 / (1.0 - 0.4);
    assert!((result - expected).abs() < 1e-10);
}

#[test]
fn harville_exacta_returns_zero_when_first_horse_wins_with_certainty() {
    // win_a >= 1.0 → denominator (1-win_a) <= 0 → guard returns 0.0
    assert_eq!(harville_exacta(1.0, 0.3), 0.0);
}

#[test]
fn harville_exacta_valid_when_only_win_a_is_below_one() {
    // win_a + win_b can exceed 1.0 as long as win_a < 1.0 (denominator is positive)
    let result = harville_exacta(0.6, 0.5);
    assert!(result > 0.0, "expected positive probability, got {result}");
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
    assert!(
        trio <= 1.0,
        "trio probability should not exceed 1.0, got {trio}"
    );
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

#[test]
fn bet_combination_encodes_type_and_code() {
    let win = BetCombination::Win(horse(3));
    assert_eq!(win.type_label(), "win");
    assert_eq!(win.combination_code(), "3");

    let place = BetCombination::Place(horse(7));
    assert_eq!(place.type_label(), "place");
    assert_eq!(place.combination_code(), "7");

    let quinella = BetCombination::Quinella(Pair::try_from((horse(5), horse(1))).unwrap());
    assert_eq!(quinella.type_label(), "quinella");
    // Pair は昇順に正規化されるため "1-5"
    assert_eq!(quinella.combination_code(), "1-5");

    let exacta = BetCombination::Exacta(OrderedPair::try_from((horse(1), horse(5))).unwrap());
    assert_eq!(exacta.type_label(), "exacta");
    assert_eq!(exacta.combination_code(), "1>5");

    let trio = BetCombination::Trio(Triple::try_from((horse(3), horse(1), horse(5))).unwrap());
    assert_eq!(trio.type_label(), "trio");
    // Triple は昇順に正規化されるため "1-3-5"
    assert_eq!(trio.combination_code(), "1-3-5");

    let trifecta = BetCombination::Trifecta(
        OrderedTriple::try_from((horse(1), horse(3), horse(5))).unwrap(),
    );
    assert_eq!(trifecta.type_label(), "trifecta");
    assert_eq!(trifecta.combination_code(), "1>3>5");
}
