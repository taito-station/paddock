use netkeiba_scraper::parse::parse_race_result;
use paddock_domain::ResultStatus;

const FIXTURE: &str = include_str!("fixtures/race_result.html");
const RACE_ID: &str = "202606030801";

// 非完走（取消）行を含む最小の結果テーブル。fixture は全馬完走のため status 異常系をここで網羅する。
const NON_FINISHER_HTML: &str = r#"
<table id="All_Result_Table"><tbody>
  <tr>
    <td class="Result_Num"><div class="Rank">1</div></td>
    <td class="Num Txt_C">5</td>
    <td class="Jockey">武豊</td>
    <td class="Trainer"><a title="友道">栗東 友道</a></td>
  </tr>
  <tr>
    <td class="Result_Num"><div class="Rank">取</div></td>
    <td class="Num Txt_C">3</td>
    <td class="Jockey">川田</td>
    <td class="Trainer"><a title="中内田">栗東 中内田</a></td>
  </tr>
</tbody></table>"#;

#[test]
fn non_finisher_has_no_position_and_status() {
    let rows = parse_race_result(NON_FINISHER_HTML, RACE_ID).expect("parse");
    assert_eq!(rows.len(), 2);
    let cancelled = rows
        .iter()
        .find(|r| r.horse_num.value() == 3)
        .expect("馬番3");
    assert_eq!(cancelled.status, ResultStatus::Cancelled);
    assert_eq!(cancelled.finishing_position, None);
    assert_eq!(
        cancelled.trainer.as_ref().map(|t| t.value()),
        Some("中内田")
    );
    let finisher = rows
        .iter()
        .find(|r| r.horse_num.value() == 5)
        .expect("馬番5");
    assert_eq!(finisher.status, ResultStatus::Finished);
    assert_eq!(finisher.finishing_position.map(|p| p.value()), Some(1));
}

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
    // 走破タイム（先頭の td.Time）が拾えていること（列順変化で別 td.Time を拾う回帰の検知）。
    assert_eq!(
        winner.time_seconds.map(|t| t.value()),
        Some(114.9), // 1:54.9
        "走破タイム 1:54.9 = 114.9 秒"
    );
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
