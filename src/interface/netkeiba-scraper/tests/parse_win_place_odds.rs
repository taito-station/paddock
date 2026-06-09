use netkeiba_scraper::parse::parse_win_place_odds;

const FIXTURE: &str = include_str!("fixtures/odds_win.json");

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
fn returns_empty_when_odds_missing() {
    // オッズ表が無い JSON（レース前）は単勝・複勝とも空。
    let json = r#"{"status":"result","data":{"official_datetime":""},"update_count":"0"}"#;
    let odds = parse_win_place_odds(json).unwrap();
    assert!(odds.win.is_empty());
    assert!(odds.place.is_empty());
}
