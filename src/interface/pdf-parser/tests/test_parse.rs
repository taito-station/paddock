use paddock_use_case::pdf_parser::PdfParser;
use pdf_parser::MutoolParser;

const SAMPLE: &[u8] = include_bytes!("../../../../samples/2026-3nakayama6.pdf");

#[test]
fn parses_sample_pdf_into_twelve_races() {
    let parser = MutoolParser;
    let races = parser.parse(SAMPLE).expect("parse sample pdf");
    assert_eq!(races.len(), 12, "expected 12 races, got {}", races.len());
}

#[test]
fn each_race_has_results() {
    let parser = MutoolParser;
    let races = parser.parse(SAMPLE).expect("parse sample pdf");
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
    let races = parser.parse(SAMPLE).expect("parse sample pdf");
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
fn detects_scratched_horse_in_race_two() {
    let parser = MutoolParser;
    let races = parser.parse(SAMPLE).expect("parse sample pdf");
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
