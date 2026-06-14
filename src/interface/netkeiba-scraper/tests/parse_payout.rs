use netkeiba_scraper::parse::parse_race_payouts;
use paddock_domain::RaceId;

const FIXTURE: &str = include_str!("fixtures/race_result.html");

fn race_id() -> RaceId {
    // 202606030801 = 2026 3回中山8日1R。result test と同じ fixture。
    RaceId::try_from("2026-3-nakayama-8-1R").unwrap()
}

// fixture の払戻ブロック（実値）:
//   単勝 6=2,270 / 複勝 6=280,4=110,11=150
//   馬連 4-6=1,170 / ワイド 4-6=430,6-11=980,4-11=180
//   馬単 6>4=3,830 / 3連複 4-6-11=1,740 / 3連単 6>4>11=22,000
//   枠連 4-5=1,200 は predict 非対象でスキップされる。
#[test]
fn parses_all_predict_bet_types() {
    let p = parse_race_payouts(FIXTURE, race_id()).expect("parse payouts");
    assert!(!p.is_empty(), "確定済み fixture は払戻を持つ");

    // 単勝
    assert_eq!(p.payoff("win", "6"), Some(2270));
    // 複勝（馬番ごとに 1 点）
    assert_eq!(p.payoff("place", "6"), Some(280));
    assert_eq!(p.payoff("place", "4"), Some(110));
    assert_eq!(p.payoff("place", "11"), Some(150));
    // 馬連（昇順）
    assert_eq!(p.payoff("quinella", "4-6"), Some(1170));
    // ワイド（昇順・3 組）
    assert_eq!(p.payoff("wide", "4-6"), Some(430));
    assert_eq!(p.payoff("wide", "6-11"), Some(980));
    assert_eq!(p.payoff("wide", "4-11"), Some(180));
    // 馬単（着順、> 連結）
    assert_eq!(p.payoff("exacta", "6>4"), Some(3830));
    // 3連複（昇順）
    assert_eq!(p.payoff("trio", "4-6-11"), Some(1740));
    // 3連単（着順）
    assert_eq!(p.payoff("trifecta", "6>4>11"), Some(22000));
}

#[test]
fn skips_wakuren_and_misses() {
    let p = parse_race_payouts(FIXTURE, race_id()).expect("parse payouts");
    // 枠連は predict 非対象なのでどの券種にも入らない（type_label を持たない）。
    // 念のため馬連キーで枠連の値(1200)が紛れていないことを確認する。
    assert_ne!(p.payoff("quinella", "4-5"), Some(1200));
    // 不的中の組合せは None。
    assert_eq!(p.payoff("quinella", "1-2"), None);
    assert_eq!(p.payoff("win", "1"), None);
}

#[test]
fn empty_html_is_unconfirmed() {
    let p = parse_race_payouts("<html><body></body></html>", race_id()).expect("parse");
    assert!(p.is_empty());
}
