//! 組合せ券種オッズ（馬連・ワイド・馬単・三連複・三連単）のパース網羅テスト（#102, #187）。
//! fixture のオッズは 2026-06-13 阪神4R の実確定値に基づく。

use netkeiba_scraper::parse::{
    parse_exacta_odds, parse_quinella_odds, parse_trifecta_odds, parse_trio_odds, parse_wide_odds,
};

const QUINELLA: &str = include_str!("fixtures/odds_quinella.json");
const WIDE: &str = include_str!("fixtures/odds_wide.json");
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
fn parses_wide_unordered_bands() {
    // ワイド(type=5)は無順序ペアに下限〜上限の帯 odds を持つ（複勝と同形）。#187
    let odds = parse_wide_odds(WIDE).expect("parse wide");
    assert_eq!(odds.len(), 3);

    // 04-07 = 7.8〜9.1（昇順キーに正規化）。下限=odds_low・上限=odds_high。
    let w = odds
        .iter()
        .find(|o| o.combination.to_key() == "4-7")
        .expect("pair 4-7");
    assert!((w.odds_low - 7.8).abs() < 1e-9, "low={}", w.odds_low);
    assert!((w.odds_high - 9.1).abs() < 1e-9, "high={}", w.odds_high);
    assert_eq!(w.popularity, Some(8));

    // 生キー "0507" が無順序キー "5-7" に正規化されることも個別に固定する。
    let mid = odds
        .iter()
        .find(|o| o.combination.to_key() == "5-7")
        .expect("pair 5-7");
    assert!((mid.odds_low - 2.3).abs() < 1e-9, "low={}", mid.odds_low);
    assert!((mid.odds_high - 2.6).abs() < 1e-9, "high={}", mid.odds_high);
    assert_eq!(mid.popularity, Some(2));

    // 01-02 のような高額帯（カンマ無し）も両端を取り込む。
    let big = odds
        .iter()
        .find(|o| o.combination.to_key() == "1-2")
        .expect("pair 1-2");
    assert!((big.odds_low - 88.4).abs() < 1e-9, "low={}", big.odds_low);
    assert!(
        (big.odds_high - 112.5).abs() < 1e-9,
        "high={}",
        big.odds_high
    );
}

#[test]
fn skips_wide_rows_missing_a_band_end() {
    // 前売り中に片端のみ（下限のみ確定・上限未確定）の行はスキップし、両端確定のみ取り込む。#114
    let json = r#"{"status":"middle","data":{"odds":{"5":{
        "0407":["7.8","9.1","8"],
        "0102":["88.4","---.-","--"]
    }}}}"#;
    let odds = parse_wide_odds(json).expect("parse");
    assert_eq!(odds.len(), 1, "片端欠落の行はスキップされる");
    assert_eq!(odds[0].combination.to_key(), "4-7");
}

#[test]
fn wide_returns_empty_when_pool_absent() {
    // 未公開（type=5 マップが無い）レース前は空で返す（エラーにしない）。
    let json = r#"{"status":"result","data":{"official_datetime":""}}"#;
    assert!(parse_wide_odds(json).unwrap().is_empty());
}

#[test]
fn wide_rejects_unexpected_or_absent_status() {
    // status="NG" / status キー欠落は fail-closed で受理しない（#100、他券種と対称）。
    assert!(parse_wide_odds(r#"{"status":"NG","data":""}"#).is_err());
    assert!(parse_wide_odds(r#"{"data":{"odds":{"5":{"0102":["1.5","2.0","1"]}}}}"#).is_err());
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
