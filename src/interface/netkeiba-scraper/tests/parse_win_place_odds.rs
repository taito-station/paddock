use netkeiba_scraper::Error;
use netkeiba_scraper::parse::parse_win_place_odds;

const FIXTURE: &str = include_str!("fixtures/odds_win.json");
// 発走前の前売り中（status="middle"）に全頭の単複オッズがそろった JSON（5 頭分）。
const FIXTURE_MIDDLE: &str = include_str!("fixtures/odds_win_middle.json");

// fixture は race_id 202605030211 の単勝・複勝オッズ JSON（17 頭分）。
#[test]
fn parses_all_win_odds() {
    let odds = parse_win_place_odds(FIXTURE).expect("parse odds");
    assert_eq!(odds.win.len(), 17);

    // 馬番昇順に整列している。
    let nums: Vec<u32> = odds.win.iter().map(|o| o.horse_num.value()).collect();
    assert_eq!(nums, (1..=17).collect::<Vec<_>>());

    // 馬番1: 7.9 / 人気3。
    let h1 = &odds.win[0];
    assert_eq!(h1.horse_num.value(), 1);
    assert!((h1.odds - 7.9).abs() < 1e-9, "odds={}", h1.odds);
    assert_eq!(h1.popularity, Some(3));

    // 馬番14: 2.9 / 人気1。
    let h14 = odds
        .win
        .iter()
        .find(|o| o.horse_num.value() == 14)
        .expect("horse 14");
    assert!((h14.odds - 2.9).abs() < 1e-9, "odds={}", h14.odds);
    assert_eq!(h14.popularity, Some(1));
}

#[test]
fn parses_all_place_odds() {
    let odds = parse_win_place_odds(FIXTURE).expect("parse odds");
    assert_eq!(odds.place.len(), 17);

    // 馬番昇順に整列している。
    let nums: Vec<u32> = odds.place.iter().map(|o| o.horse_num.value()).collect();
    assert_eq!(nums, (1..=17).collect::<Vec<_>>());

    // 馬番1: 複勝 2.6 - 4.1 / 人気5。
    let p1 = &odds.place[0];
    assert_eq!(p1.horse_num.value(), 1);
    assert!((p1.odds_low - 2.6).abs() < 1e-9, "low={}", p1.odds_low);
    assert!((p1.odds_high - 4.1).abs() < 1e-9, "high={}", p1.odds_high);
    assert_eq!(p1.popularity, Some(5));

    // 馬番14: 複勝 1.3 - 1.5 / 人気1。
    let p14 = odds
        .place
        .iter()
        .find(|o| o.horse_num.value() == 14)
        .expect("horse 14");
    assert!((p14.odds_low - 1.3).abs() < 1e-9, "low={}", p14.odds_low);
    assert!((p14.odds_high - 1.5).abs() < 1e-9, "high={}", p14.odds_high);
    assert_eq!(p14.popularity, Some(1));
}

#[test]
fn parses_odds_when_status_is_middle() {
    // 前売り中（status="middle"）でも全頭の単複オッズを取り込める。
    let odds = parse_win_place_odds(FIXTURE_MIDDLE).expect("parse middle odds");
    assert_eq!(odds.win.len(), 5);
    assert_eq!(odds.place.len(), 5);

    // 馬番5: 単勝 6.5 / 人気2、複勝 2.0 - 3.0。
    let w5 = odds
        .win
        .iter()
        .find(|o| o.horse_num.value() == 5)
        .expect("win horse 5");
    assert!((w5.odds - 6.5).abs() < 1e-9, "odds={}", w5.odds);
    assert_eq!(w5.popularity, Some(2));

    let p5 = odds
        .place
        .iter()
        .find(|o| o.horse_num.value() == 5)
        .expect("place horse 5");
    assert!((p5.odds_low - 2.0).abs() < 1e-9, "low={}", p5.odds_low);
    assert!((p5.odds_high - 3.0).abs() < 1e-9, "high={}", p5.odds_high);
}

#[test]
fn rejects_unexpected_status() {
    // 未掲載・対象外（status="NG"）はエラーにする。
    let json = r#"{"status":"NG","data":"","update_count":"0","reason":"history odds empty"}"#;
    let err = parse_win_place_odds(json).expect_err("NG status should error");
    // メッセージ文言だけでなくエラー種別（Parse）も固定する。
    assert!(matches!(err, Error::Parse(_)), "err={err}");
    assert!(err.to_string().contains("status=NG"), "err={err}");
}

#[test]
fn parses_when_status_key_absent() {
    // status キーが無い JSON は受理チェックをすり抜け、オッズがあればそのまま取り込む
    // （fail-open）。この既存挙動を固定し、将来の API 仕様変更時の回帰検知にする。
    let json = r#"{"data":{"odds":{"1":{"03":["4.2","0.0","1"]}}}}"#;
    let odds = parse_win_place_odds(json).expect("absent status should parse");
    assert_eq!(odds.win.len(), 1);
    assert_eq!(odds.win[0].horse_num.value(), 3);
    assert!((odds.win[0].odds - 4.2).abs() < 1e-9, "odds={}", odds.win[0].odds);
}

#[test]
fn middle_skips_unpriced_rows() {
    // 前売り中（middle）で一部馬のオッズが未確定（"---.-"）の場合、その行だけスキップし
    // 確定済みの馬は取り込む。
    let json = r#"{"status":"middle","data":{"odds":{
        "1":{"01":["3.5","0.0","1"],"02":["---.-","0.0","0"]},
        "2":{"01":["1.5","2.1","1"],"02":["---.-","---.-","0"]}
    }}}"#;
    let odds = parse_win_place_odds(json).expect("middle with unpriced rows should parse");
    // 単勝・複勝とも確定済みの馬番1のみが残る。
    assert_eq!(odds.win.len(), 1);
    assert_eq!(odds.win[0].horse_num.value(), 1);
    assert_eq!(odds.place.len(), 1);
    assert_eq!(odds.place[0].horse_num.value(), 1);
}

#[test]
fn returns_empty_when_odds_missing() {
    // オッズ表が無い JSON（レース前）は単勝・複勝とも空。
    let json = r#"{"status":"result","data":{"official_datetime":""},"update_count":"0"}"#;
    let odds = parse_win_place_odds(json).unwrap();
    assert!(odds.win.is_empty());
    assert!(odds.place.is_empty());
}
