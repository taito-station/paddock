use chrono::NaiveDate;
use entry_parser::MutoolEntryParser;
use paddock_domain::{Surface, Venue};
use paddock_use_case::entry_parser::EntryParser;
use std::path::PathBuf;

/// テスト用の出馬表 PDF を返す。
///
/// JRA 著作物のためリポジトリには含めない（`samples/*.pdf` は gitignore 済み）。
/// 出馬表 PDF は結果 PDF と違い安定した公開 URL が無いため、ローカルに存在すれば
/// それを使い、無ければ取得せずスキップする（`None` を返す）。
fn sample_entry_pdf() -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../samples/2026-3nakayama8-entries.pdf");
    match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!("skip: entry sample PDF が不在のためスキップ ({})", path.display());
            None
        }
    }
}

// The entry PDF has no date text; the caller supplies it (derived from the source
// filename). This sample meeting (3 回中山 8 日) ran on 2026-04-19.
fn sample_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 4, 19).unwrap()
}

#[test]
fn parses_sample_entry_pdf_into_twelve_race_cards() {
    let parser = MutoolEntryParser;
    let Some(sample) = sample_entry_pdf() else {
        return;
    };
    let cards = parser
        .parse(&sample, sample_date())
        .expect("parse sample entry pdf");
    assert_eq!(
        cards.len(),
        12,
        "expected 12 race cards, got {}",
        cards.len()
    );
}

#[test]
fn each_race_card_has_entries() {
    let parser = MutoolEntryParser;
    let Some(sample) = sample_entry_pdf() else {
        return;
    };
    let cards = parser.parse(&sample, sample_date()).expect("parse");
    for card in &cards {
        assert!(
            !card.entries.is_empty(),
            "race {} has no entries",
            card.race_id
        );
    }
}

#[test]
fn race1_metadata() {
    let parser = MutoolEntryParser;
    let Some(sample) = sample_entry_pdf() else {
        return;
    };
    let cards = parser.parse(&sample, sample_date()).expect("parse");
    let r1 = cards
        .iter()
        .find(|c| c.race_num == 1)
        .expect("race 1 not found");
    assert_eq!(r1.date, sample_date());
    assert_eq!(r1.distance, 1800);
    assert_eq!(r1.surface, Surface::Dirt);
    assert_eq!(r1.venue, Venue::Nakayama);
    assert_eq!(r1.round, 3);
    assert_eq!(r1.day, 8);
    assert_eq!(r1.entries.len(), 12);
}

#[test]
fn race11_has_eighteen_entries() {
    let parser = MutoolEntryParser;
    let Some(sample) = sample_entry_pdf() else {
        return;
    };
    let cards = parser.parse(&sample, sample_date()).expect("parse");
    let r11 = cards
        .iter()
        .find(|c| c.race_num == 11)
        .expect("race 11 not found");
    assert_eq!(r11.surface, Surface::Turf);
    assert_eq!(r11.entries.len(), 18, "Satsuki Sho should have 18 entries");
}

#[test]
fn race1_horse_names_and_jockeys() {
    let parser = MutoolEntryParser;
    let Some(sample) = sample_entry_pdf() else {
        return;
    };
    let cards = parser.parse(&sample, sample_date()).expect("parse");
    let r1 = cards
        .iter()
        .find(|c| c.race_num == 1)
        .expect("race 1 not found");
    let h1 = r1
        .entries
        .iter()
        .find(|e| e.horse_num.value() == 1)
        .expect("horse 1 not found");
    assert_eq!(h1.horse_name.value(), "ストーリーオブラブ");
    assert_eq!(h1.gate_num.value(), 1);
    let jockey = h1.jockey.as_ref().expect("jockey missing for horse 1");
    assert!(
        jockey.value().contains("小林"),
        "jockey should contain 小林, got: {}",
        jockey.value()
    );
}
