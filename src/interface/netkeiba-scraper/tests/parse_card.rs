use chrono::{NaiveDate, NaiveTime};
use netkeiba_scraper::parse::parse_card;
use paddock_domain::{RaceClass, Surface, Venue};

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
    // レースクラス（#345）。<title>「安田記念(G1)」のグレード表記から G1 と判定する。
    assert_eq!(card.race_class, Some(RaceClass::G1));

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

// グレードが <title> に無いレースは RaceData02 の条件表記からクラスを取る（#345）。安田記念
// fixture の title グレード「安田記念(G1)」を外し、RaceData02 の「オープン」を「３歳未勝利」に
// 差し替えて、非重賞クラス抽出経路（div.RaceData02）が働き Maiden を返すことを検証する。
// （この経路は G1裏＝別場の非重賞検出の前提。title のみで確定する G1 ケースだけでは未検証になる）
#[test]
fn race_class_from_racedata02_when_title_has_no_grade() {
    let html = FIXTURE
        .replace("安田記念(G1)", "テスト特別")
        .replace("オープン", "３歳未勝利");
    let card = parse_card(&html, RACE_ID).expect("parse card");
    assert_eq!(card.race_class, Some(RaceClass::Maiden));
    // 他メタは通常どおり取れる。
    assert_eq!(card.distance, 1600);
    assert_eq!(card.entries.len(), 17);
}

// G1 以外のグレード（G2）も <title> の括弧付き表記から取れる（#345）。fixture の
// 「安田記念(G1)」を「○○記念(G2)」に差し替えて title→グレードの経路を G1 以外でも検証する。
#[test]
fn race_class_reads_g2_grade_from_title() {
    let html = FIXTURE.replace("安田記念(G1)", "○○記念(G2)");
    let card = parse_card(&html, RACE_ID).expect("parse card");
    assert_eq!(card.race_class, Some(RaceClass::G2));
}

// n勝クラスは RaceData02 に全角数字（「３勝クラス」）でレンダされるため、全角のまま Win3 を
// 取れることを parse 経由で検証する（#345）。G1裏の大半は n勝クラスのアンダーカードなので、
// この経路が全角数字で機能しないと 🎯裏 が実質点かない。title グレードを外し、RaceData02 の
// 「オープン」を全角数字の「３勝クラス」へ差し替えて末端まで確認する。
#[test]
fn race_class_reads_fullwidth_win_class_from_racedata02() {
    let html = FIXTURE
        .replace("安田記念(G1)", "テスト特別")
        .replace("オープン", "３勝クラス");
    let card = parse_card(&html, RACE_ID).expect("parse card");
    assert_eq!(card.race_class, Some(RaceClass::Win3));
}

// 発走時刻表記（「HH:MM発走」）が無い HTML では post_time が best-effort で None になり、
// それでもカード自体は他項目から組める（#235）。発走時刻トークンだけを除いて再現する
// （「発走」全置換だと将来 fixture の別箇所に「発走」が増えたとき意図せず消えるため、
// post-time の "15:40発走" → "15:40" に限定する）。
#[test]
fn post_time_is_none_when_absent() {
    let html = FIXTURE.replace("15:40発走", "15:40");
    let card = parse_card(&html, RACE_ID).expect("parse card");
    assert_eq!(card.post_time, None, "発走表記が無ければ post_time は None");
    // 発走時刻が無くても他のメタ・出走馬は通常どおり取れる（カード保存を止めない）。
    assert_eq!(card.distance, 1600);
    assert_eq!(card.entries.len(), 17);
}
