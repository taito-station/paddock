//! 組合せ券種オッズ（馬連・馬単・三連複・三連単）のパース網羅テスト（#102）。
//! fixture のオッズは 2026-06-13 阪神4R の実確定値に基づく。

use netkeiba_scraper::parse::{
    parse_exacta_odds, parse_quinella_odds, parse_trifecta_odds, parse_trio_odds,
};

const QUINELLA: &str = include_str!("fixtures/odds_quinella.json");
const EXACTA: &str = include_str!("fixtures/odds_exacta.json");
const TRIO: &str = include_str!("fixtures/odds_trio.json");
const TRIFECTA: &str = include_str!("fixtures/odds_trifecta.json");

#[test]
fn parses_quinella_unordered_pairs() {
    let odds = parse_quinella_odds(QUINELLA).expect("parse quinella");
    assert_eq!(odds.len(), 3);

    // 04-07 = 21.6（昇順キーに正規化）。
    let q = odds
        .iter()
        .find(|o| o.combination.to_key() == "4-7")
        .expect("pair 4-7");
    assert!((q.odds - 21.6).abs() < 1e-9, "odds={}", q.odds);
    assert_eq!(q.popularity, Some(9));

    // カンマ区切りの高額オッズ "1,141.1" を正しくパースする。
    let big = odds
        .iter()
        .find(|o| o.combination.to_key() == "1-2")
        .expect("pair 1-2");
    assert!((big.odds - 1141.1).abs() < 1e-6, "odds={}", big.odds);
}

#[test]
fn parses_exacta_ordered_pairs() {
    let odds = parse_exacta_odds(EXACTA).expect("parse exacta");
    assert_eq!(odds.len(), 2);

    // 07→04 = 31.0 と 04→07 = 58.6 は別物（順序を保持する）。
    let fwd = odds
        .iter()
        .find(|o| o.combination.to_key() == "7>4")
        .expect("7>4");
    assert!((fwd.odds - 31.0).abs() < 1e-9, "odds={}", fwd.odds);
    let rev = odds
        .iter()
        .find(|o| o.combination.to_key() == "4>7")
        .expect("4>7");
    assert!((rev.odds - 58.6).abs() < 1e-9, "odds={}", rev.odds);
}

#[test]
fn parses_trio_unordered_triples() {
    let odds = parse_trio_odds(TRIO).expect("parse trio");
    assert_eq!(odds.len(), 2);

    // 04-07-13 = 32.9（昇順キー）。
    let t = odds
        .iter()
        .find(|o| o.combination.to_key() == "4-7-13")
        .expect("triple 4-7-13");
    assert!((t.odds - 32.9).abs() < 1e-9, "odds={}", t.odds);
    assert_eq!(t.popularity, Some(9));
}

#[test]
fn parses_trifecta_ordered_triples() {
    let odds = parse_trifecta_odds(TRIFECTA).expect("parse trifecta");
    assert_eq!(odds.len(), 2);

    // 07→04→13 = 154.6（順序を保持）。
    let t = odds
        .iter()
        .find(|o| o.combination.to_key() == "7>4>13")
        .expect("ordered triple 7>4>13");
    assert!((t.odds - 154.6).abs() < 1e-9, "odds={}", t.odds);
    assert_eq!(t.popularity, Some(40));
}

#[test]
fn skips_unpriced_combo_rows() {
    // 前売り中に一部の組合せが未確定（"---.-"）の行はスキップし、確定済みのみ取り込む。
    let json = r#"{"status":"middle","data":{"odds":{"4":{
        "0407":["21.6","0.0","9"],
        "0102":["---.-","0.0","--"]
    }}}}"#;
    let odds = parse_quinella_odds(json).expect("parse");
    assert_eq!(odds.len(), 1, "未確定行はスキップされる");
    assert_eq!(odds[0].combination.to_key(), "4-7");
}

#[test]
fn returns_empty_when_pool_absent() {
    // 未公開（券種マップが無い）レース前は空で返す（エラーにしない）。
    let json = r#"{"status":"result","data":{"official_datetime":""}}"#;
    assert!(parse_quinella_odds(json).unwrap().is_empty());
    assert!(parse_trifecta_odds(json).unwrap().is_empty());
}

#[test]
fn rejects_unexpected_status() {
    // 未掲載・対象外（status="NG"）は組合せ券種でもエラーにする。
    let json = r#"{"status":"NG","data":"","reason":"history odds empty"}"#;
    assert!(parse_quinella_odds(json).is_err());
    assert!(parse_trio_odds(json).is_err());
}

#[test]
fn rejects_when_status_key_absent() {
    // status キー欠落は組合せ券種でも受理しない（fail-closed, #100）。単複側と対称に、
    // 共通の status 検証（parse_validated_root）が組合せ券種パーサでも効くことを固定する。
    let json = r#"{"data":{"odds":{"4":{"0102":["12.3","0.0","1"]}}}}"#;
    assert!(parse_quinella_odds(json).is_err());
    assert!(parse_trio_odds(json).is_err());
}
