use netkeiba_scraper::parse::parse_win_odds;

const FIXTURE: &str = include_str!("fixtures/odds_win.json");

// fixture は race_id 202605030211 の単勝オッズ JSON（17 頭分）。
#[test]
fn parses_all_win_odds() {
    let odds = parse_win_odds(FIXTURE).expect("parse win odds");
    assert_eq!(odds.len(), 17);

    // 馬番昇順に整列している。
    let nums: Vec<u32> = odds.iter().map(|o| o.horse_num.value()).collect();
    assert_eq!(nums, (1..=17).collect::<Vec<_>>());

    // 馬番1: 7.9 / 人気3。
    let h1 = &odds[0];
    assert_eq!(h1.horse_num.value(), 1);
    assert!((h1.odds - 7.9).abs() < 1e-9, "odds={}", h1.odds);
    assert_eq!(h1.popularity, Some(3));

    // 馬番14: 2.9 / 人気1。
    let h14 = odds
        .iter()
        .find(|o| o.horse_num.value() == 14)
        .expect("horse 14");
    assert!((h14.odds - 2.9).abs() < 1e-9, "odds={}", h14.odds);
    assert_eq!(h14.popularity, Some(1));
}

#[test]
fn returns_empty_when_odds_missing() {
    // 単勝表が無い JSON（レース前）は空 Vec。
    let json = r#"{"status":"result","data":{"official_datetime":""},"update_count":"0"}"#;
    assert!(parse_win_odds(json).unwrap().is_empty());
}
