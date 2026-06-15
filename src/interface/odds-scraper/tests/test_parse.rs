//! Fixture-based tests for the JRA odds HTML parsers and assembly step.
//!
//! Fixtures under `tests/fixtures/` are representative of JRA's published odds
//! table layout (see crate docs / ADR 0001). The live POST/cname navigation is
//! not exercised here — only the parsing and assembly, which is the verified
//! core of this crate.

use odds_scraper::{OddsPages, assemble, parse};
use paddock_domain::{
    HorseNum, OddsValue, OrderedPair, OrderedTriple, Pair, PlaceOdds, RaceId, Triple,
};

const WIN_PLACE: &str = include_str!("fixtures/win_place.html");
const QUINELLA: &str = include_str!("fixtures/quinella.html");
const EXACTA: &str = include_str!("fixtures/exacta.html");
const TRIO: &str = include_str!("fixtures/trio.html");
const TRIFECTA: &str = include_str!("fixtures/trifecta.html");

fn hn(n: u32) -> HorseNum {
    HorseNum::try_from(n).unwrap()
}

#[test]
fn parses_win_and_place_skipping_unpublished_and_scratched() {
    let (win, place) = parse::parse_win_place(WIN_PLACE).expect("parse win/place");

    // Horses 1..=3 have odds; 4 (---) and 5 (取消) are skipped.
    assert_eq!(win.len(), 3);
    assert_eq!(place.len(), 3);
    assert_eq!(win[&hn(1)].value(), 2.5);
    assert_eq!(win[&hn(3)].value(), 12.0);
    assert!(!win.contains_key(&hn(4)));
    assert!(!win.contains_key(&hn(5)));

    let band = place[&hn(2)];
    assert_eq!(band.low.value(), 1.8);
    assert_eq!(band.high.value(), 2.4);
}

#[test]
fn parses_quinella_normalising_order() {
    let map = parse::parse_quinella(QUINELLA).expect("parse quinella");
    // 3 published rows; the "3 - 4" row (---) is skipped.
    assert_eq!(map.len(), 3);
    let key = Pair::try_from((hn(2), hn(1))).unwrap(); // unordered: same as 1-2
    assert_eq!(map[&key].value(), 8.4);
}

#[test]
fn parses_exacta_preserving_order() {
    let map = parse::parse_exacta(EXACTA).expect("parse exacta");
    assert_eq!(map.len(), 3);
    let one_two = OrderedPair::try_from((hn(1), hn(2))).unwrap();
    let two_one = OrderedPair::try_from((hn(2), hn(1))).unwrap();
    assert_eq!(map[&one_two].value(), 16.8);
    assert_eq!(map[&two_one].value(), 19.5);
    assert_ne!(map[&one_two].value(), map[&two_one].value());
}

#[test]
fn parses_trio_unordered() {
    let map = parse::parse_trio(TRIO).expect("parse trio");
    assert_eq!(map.len(), 3);
    let key = Triple::try_from((hn(3), hn(1), hn(2))).unwrap(); // unordered: same as 1-2-3
    assert_eq!(map[&key].value(), 45.7);
}

#[test]
fn parses_trifecta_with_thousands_separator() {
    let map = parse::parse_trifecta(TRIFECTA).expect("parse trifecta");
    assert_eq!(map.len(), 3);
    let key = OrderedTriple::try_from((hn(3), hn(2), hn(1))).unwrap();
    assert_eq!(map[&key].value(), 1234.5);
}

#[test]
fn assemble_combines_all_bet_types() {
    let race_id = RaceId::try_from("202603nakayama6R11").unwrap();
    let pages = OddsPages {
        win_place: Some(WIN_PLACE.to_string()),
        quinella: Some(QUINELLA.to_string()),
        exacta: Some(EXACTA.to_string()),
        trio: Some(TRIO.to_string()),
        trifecta: Some(TRIFECTA.to_string()),
    };
    let odds = assemble(race_id, &pages).expect("assemble");
    assert!(!odds.is_empty());
    assert_eq!(odds.win.len(), 3);
    assert_eq!(odds.place.len(), 3);
    assert_eq!(odds.quinella.len(), 3);
    assert_eq!(odds.exacta.len(), 3);
    assert_eq!(odds.trio.len(), 3);
    assert_eq!(odds.trifecta.len(), 3);
}

#[test]
fn assemble_with_no_pages_is_empty() {
    let race_id = RaceId::try_from("202603nakayama6R11").unwrap();
    let odds = assemble(race_id, &OddsPages::default()).expect("assemble empty");
    assert!(odds.is_empty());
}

// --- Domain guard coverage (value objects reject invalid input) ---

#[test]
fn odds_value_rejects_below_one_and_non_finite() {
    assert!(OddsValue::try_from(0.9).is_err());
    assert!(OddsValue::try_from(f64::NAN).is_err());
    assert!(OddsValue::try_from(1.0).is_ok());
}

#[test]
fn combination_keys_reject_duplicate_horses() {
    assert!(Pair::try_from((hn(3), hn(3))).is_err());
    assert!(OrderedPair::try_from((hn(3), hn(3))).is_err());
    assert!(Triple::try_from((hn(1), hn(2), hn(2))).is_err());
    assert!(OrderedTriple::try_from((hn(1), hn(1), hn(2))).is_err());
}

#[test]
fn place_band_rejects_inverted_range() {
    let low = OddsValue::try_from(2.0).unwrap();
    let high = OddsValue::try_from(1.0).unwrap();
    assert!(PlaceOdds::try_from((low, high)).is_err());
}

#[test]
fn combo_with_wrong_arity_errors() {
    // A 馬連 (2-horse) parser fed a 3-horse combination cell must error.
    let html = r#"<table><tr><td class="num">1 - 2 - 3</td><td class="odds">5.0</td></tr></table>"#;
    assert!(parse::parse_quinella(html).is_err());
}
