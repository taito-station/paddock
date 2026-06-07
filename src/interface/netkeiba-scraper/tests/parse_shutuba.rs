use netkeiba_scraper::parse::parse_shutuba;

const FIXTURE: &str = include_str!("fixtures/shutuba.html");

// fixture は 2026 安田記念(race_id 202605030211)の出馬表テーブル（17 頭立て）。
#[test]
fn extracts_all_runners_in_gate_order() {
    let runners = parse_shutuba(FIXTURE).expect("parse");
    assert_eq!(runners.len(), 17);

    // 馬番 1 番・2 番の馬名と horse_id を厳密に固定。
    assert_eq!(runners[0].horse_num.value(), 1);
    assert_eq!(runners[0].horse_name.value(), "レーベンスティール");
    assert_eq!(runners[0].horse_id.value(), "2020102078");

    assert_eq!(runners[1].horse_num.value(), 2);
    assert_eq!(runners[1].horse_name.value(), "ロングラン");
    assert_eq!(runners[1].horse_id.value(), "2018104708");

    // 馬番は 1..=17 が漏れなく昇順で並ぶ。
    let nums: Vec<u32> = runners.iter().map(|r| r.horse_num.value()).collect();
    assert_eq!(nums, (1..=17).collect::<Vec<_>>());
}
