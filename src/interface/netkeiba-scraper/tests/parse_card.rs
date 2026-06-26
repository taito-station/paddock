use chrono::{NaiveDate, NaiveTime};
use netkeiba_scraper::parse::parse_card;
use paddock_domain::{Surface, Venue};

const FIXTURE: &str = include_str!("fixtures/shutuba_card.html");
const RACE_ID: &str = "202605030211";

// fixture は 2026 安田記念(race_id 202605030211, 3回東京2日11R)の出馬表（芝1600m, 17頭）。
#[test]
fn parses_card_meta_and_entries() {
    let card = parse_card(FIXTURE, RACE_ID).expect("parse card");

    assert_eq!(card.venue, Venue::Tokyo);
    assert_eq!(card.round, 3);
    assert_eq!(card.day, 2);
    assert_eq!(card.race_num, 11);
    assert_eq!(card.surface, Surface::Turf);
    assert_eq!(card.distance, 1600);
    assert_eq!(card.date, NaiveDate::from_ymd_opt(2026, 6, 7).unwrap());
    // 発走時刻（#235）。RaceData01「15:40発走」から抽出する。
    assert_eq!(card.post_time, NaiveTime::from_hms_opt(15, 40, 0));

    assert_eq!(card.entries.len(), 17);

    // 1 番: 枠1・馬番1・馬名レーベンスティール・騎手戸崎圭。
    let first = &card.entries[0];
    assert_eq!(first.gate_num.value(), 1);
    assert_eq!(first.horse_num.value(), 1);
    assert_eq!(first.horse_name.value(), "レーベンスティール");
    assert_eq!(first.jockey.as_ref().map(|j| j.value()), Some("戸崎圭"));
    // 調教師（#74）。td.Trainer の title 属性から抽出する。
    assert_eq!(first.trainer.as_ref().map(|t| t.value()), Some("田中博"));
    // 斤量（#135）。性齢セル直後の td(class=Txt_C) から抽出する。
    assert_eq!(first.weight_carried, Some(58.0));
    // horse_id は近走取り込み（#103）の再利用キー。同じ /horse/ リンク（href）から抽出する。
    assert_eq!(
        first.horse_id.as_ref().map(|h| h.value()),
        Some("2020102078")
    );

    // 馬番が 1..=17 で漏れなく並ぶ。
    let nums: Vec<u32> = card.entries.iter().map(|e| e.horse_num.value()).collect();
    assert_eq!(nums, (1..=17).collect::<Vec<_>>());
}

// 発走時刻表記（「HH:MM発走」）が無い HTML では post_time が best-effort で None になり、
// それでもカード自体は他項目から組める（#235）。fixture から「発走」アンカーを除いて再現する。
#[test]
fn post_time_is_none_when_absent() {
    let html = FIXTURE.replace("発走", "");
    let card = parse_card(&html, RACE_ID).expect("parse card");
    assert_eq!(card.post_time, None, "発走表記が無ければ post_time は None");
    // 発走時刻が無くても他のメタ・出走馬は通常どおり取れる（カード保存を止めない）。
    assert_eq!(card.distance, 1600);
    assert_eq!(card.entries.len(), 17);
}
