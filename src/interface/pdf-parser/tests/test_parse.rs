use paddock_use_case::pdf_parser::PdfParser;
use pdf_parser::MutoolParser;

#[path = "../../sample_pdf_fixture.rs"]
mod fixture;

#[test]
fn parses_sample_pdf_into_twelve_races() {
    let parser = MutoolParser;
    let Some(sample) = fixture::sample_result_pdf() else {
        return;
    };
    let races = parser.parse(&sample).expect("parse sample pdf");
    assert_eq!(races.len(), 12, "expected 12 races, got {}", races.len());
}

#[test]
fn each_race_has_results() {
    let parser = MutoolParser;
    let Some(sample) = fixture::sample_result_pdf() else {
        return;
    };
    let races = parser.parse(&sample).expect("parse sample pdf");
    for race in &races {
        assert!(
            !race.results.is_empty(),
            "race {} has no results",
            race.race_id
        );
    }
}

#[test]
fn race_metadata_for_first_race() {
    let parser = MutoolParser;
    let Some(sample) = fixture::sample_result_pdf() else {
        return;
    };
    let races = parser.parse(&sample).expect("parse sample pdf");
    let r1 = races
        .iter()
        .find(|r| r.race_num == 1)
        .expect("race 1 not found");
    assert_eq!(r1.distance, 1200);
    assert_eq!(r1.surface, paddock_domain::Surface::Dirt);
    assert_eq!(r1.venue, paddock_domain::Venue::Nakayama);
    assert_eq!(r1.round, 3);
    assert_eq!(r1.day, 6);
}

#[test]
fn jockey_column_is_clean_and_separated_from_owner() {
    let parser = MutoolParser;
    let Some(sample) = fixture::sample_result_pdf() else {
        return;
    };
    let races = parser.parse(&sample).expect("parse sample pdf");

    // 既知の騎手が馬主・牧場名の混入なくクリーンに取れる（stext 実測で確定した値）。
    let r1 = races
        .iter()
        .find(|r| r.race_num == 1)
        .expect("race 1 not found");
    let jockey_of = |hn: u32| -> Option<String> {
        r1.results
            .iter()
            .find(|res| res.horse_num.value() == hn)
            .and_then(|res| res.jockey.as_ref().map(|j| j.value().to_string()))
    };
    assert_eq!(jockey_of(9).as_deref(), Some("横山和生")); // ロードトライデント
    assert_eq!(jockey_of(6).as_deref(), Some("田辺裕信")); // ニンジャトットリ

    // 右レース列（馬主が size6 で size 分離が効かず、x 帯境界で切る必要がある）の検証。
    // 修正前はそれぞれ `横山典弘秋元` / `横山武史西山` と馬主サーネームが混入していた。
    let r2 = races.iter().find(|r| r.race_num == 2).expect("race 2 not found");
    let jockey_of_r2 = |hn: u32| -> Option<String> {
        r2.results
            .iter()
            .find(|res| res.horse_num.value() == hn)
            .and_then(|res| res.jockey.as_ref().map(|j| j.value().to_string()))
    };
    assert_eq!(jockey_of_r2(6).as_deref(), Some("横山典弘"));
    assert_eq!(jockey_of_r2(2).as_deref(), Some("横山武史"));

    // 全レース・全行で馬主/調教師/牧場フラグメントや純数字（斤量誤分類）が混入しない。
    for race in &races {
        for res in &race.results {
            let Some(j) = &res.jockey else { continue };
            let v = j.value();
            assert!(
                !v.chars().any(|c| matches!(c, '氏' | '\u{FFFD}')),
                "jockey '{v}' に馬主/置換文字が混入 (race {}, horse {})",
                race.race_num,
                res.horse_num.value()
            );
            assert!(
                !v.contains("牧場") && !v.contains("ファーム"),
                "jockey '{v}' に牧場名が混入 (race {})",
                race.race_num
            );
            assert!(
                !v.chars().all(|c| c.is_ascii_digit()),
                "jockey '{v}' が純数字（斤量誤分類） (race {})",
                race.race_num
            );
        }
    }
}

#[test]
fn trainer_column_is_clean_and_populated() {
    let parser = MutoolParser;
    let Some(sample) = fixture::sample_result_pdf() else {
        return;
    };
    let races = parser.parse(&sample).expect("parse sample pdf");

    // 既知の調教師がフルネームでクリーンに取れる（stext 実測で確定。jockey と同じ馬で対応）。
    let r1 = races
        .iter()
        .find(|r| r.race_num == 1)
        .expect("race 1 not found");
    let trainer_of = |hn: u32| -> Option<String> {
        r1.results
            .iter()
            .find(|res| res.horse_num.value() == hn)
            .and_then(|res| res.trainer.as_ref().map(|t| t.value().to_string()))
    };
    assert_eq!(trainer_of(9).as_deref(), Some("千葉直人")); // ロードトライデント
    assert_eq!(trainer_of(6).as_deref(), Some("松永康利")); // ニンジャトットリ

    // 調教師がレース全体でおおむね埋まること（母数充足の最低保証。出走取消等で一部 None は許容）。
    let filled = r1.results.iter().filter(|r| r.trainer.is_some()).count();
    assert!(
        filled * 10 >= r1.results.len() * 8, // 8 割以上（整数除算の切り捨てを避ける）
        "race 1: trainer 充足 {filled}/{} が想定より少ない",
        r1.results.len()
    );

    // 全レース・全行で馬主(氏)/置換文字/牧場フラグメント/純数字が調教師に混入しない。
    for race in &races {
        for res in &race.results {
            let Some(t) = &res.trainer else { continue };
            let v = t.value();
            assert!(
                !v.chars().any(|c| matches!(c, '氏' | '\u{FFFD}')),
                "trainer '{v}' に馬主/置換文字が混入 (race {}, horse {})",
                race.race_num,
                res.horse_num.value()
            );
            assert!(
                !v.contains("牧場") && !v.contains("ファーム"),
                "trainer '{v}' に牧場名が混入 (race {})",
                race.race_num
            );
            // 調教師名は漢字のみ。牧場列(x 帯のすぐ右)が混入すると仮名混じり地名
            // （新ひだか/ノーザンファーム 等）が紛れるため、平仮名・片仮名の不在で検知する。
            assert!(
                !v.chars()
                    .any(|c| ('\u{3040}'..='\u{30FF}').contains(&c)),
                "trainer '{v}' に仮名が混入（牧場名混入の疑い） (race {})",
                race.race_num
            );
            assert!(
                !v.chars().any(|c| c.is_ascii_alphanumeric()),
                "trainer '{v}' に ASCII 英数字が混入 (レコード標示 RC・純数字等) (race {})",
                race.race_num
            );
        }
    }
}

#[test]
fn detects_scratched_horse_in_race_two() {
    let parser = MutoolParser;
    let Some(sample) = fixture::sample_result_pdf() else {
        return;
    };
    let races = parser.parse(&sample).expect("parse sample pdf");
    let r2 = races
        .iter()
        .find(|r| r.race_num == 2)
        .expect("race 2 not found");
    let scratched: Vec<&_> = r2
        .results
        .iter()
        .filter(|r| r.finishing_position.is_none())
        .collect();
    assert!(
        !scratched.is_empty(),
        "expected at least one scratched horse in race 2"
    );
}
