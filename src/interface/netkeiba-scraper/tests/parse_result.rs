use netkeiba_scraper::parse::parse_race_result;
use paddock_domain::ResultStatus;

const FIXTURE: &str = include_str!("fixtures/race_result.html");
const RACE_ID: &str = "202606030801";

// fixture は 2026 3回中山8日1R の結果（12 頭, 全馬完走）。
#[test]
fn parses_all_finishers() {
    let rows = parse_race_result(FIXTURE, RACE_ID).expect("parse result");
    assert_eq!(rows.len(), 12, "12 頭");
    // 全馬 1〜12 着が揃う（完走）。
    for r in &rows {
        assert_eq!(r.status, ResultStatus::Finished);
        assert!(r.finishing_position.is_some());
    }
}

#[test]
fn winner_has_clean_jockey_and_trainer_abbrev() {
    let rows = parse_race_result(FIXTURE, RACE_ID).expect("parse result");
    // 1 着 = 馬番 6（ナムラハリス）。jockey/trainer は netkeiba 略名（entry と同表記）。
    let winner = rows
        .iter()
        .find(|r| r.finishing_position.map(|p| p.value()) == Some(1))
        .expect("1着");
    assert_eq!(winner.horse_num.value(), 6);
    assert_eq!(winner.jockey.as_ref().map(|j| j.value()), Some("原"));
    assert_eq!(winner.trainer.as_ref().map(|t| t.value()), Some("宮地"));
    assert_eq!(winner.popularity, Some(6));
    assert_eq!(winner.odds, Some(22.7));
}

#[test]
fn jockey_and_trainer_have_no_owner_contamination() {
    // 結果ページ由来の jockey/trainer は略名で、馬主名・牧場名が混入しないこと
    // （PDF 経路の汚染「横山武史ライオンレースホース」を解消する目的）。
    let rows = parse_race_result(FIXTURE, RACE_ID).expect("parse result");
    for r in &rows {
        if let Some(j) = &r.jockey {
            let v = j.value();
            assert!(
                !v.contains("レーシング") && !v.contains("ファーム") && v.chars().count() <= 8,
                "jockey '{v}' に馬主/牧場混入の疑い"
            );
        }
        if let Some(t) = &r.trainer {
            let v = t.value();
            assert!(
                !v.contains("栗東") && !v.contains("美浦") && v.chars().count() <= 8,
                "trainer '{v}' に所属/馬主混入の疑い"
            );
        }
    }
}
