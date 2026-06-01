//! Offline tests for meeting-day discovery + exclusive control.
//!
//! JRA is unreachable from CI, so the fetcher/parser/repository are mocked.
//! These cover the URL/key derivation and the skip / not-found / ingest paths.

use std::sync::Mutex;

use chrono::NaiveDate;
use paddock_domain::horse_result::{FinishingPosition, GateNum, HorseName, HorseNum, ResultStatus};
use paddock_domain::{HorseResult, JockeyName, Race, RaceCard, RaceId, Surface, Venue};
use paddock_use_case::dto::pdf::fetch::{FetchMeetingOutcome, MeetingSpec};
use paddock_use_case::pdf_fetcher::PdfFetcher;
use paddock_use_case::pdf_parser::PdfParser;
use paddock_use_case::repository::{
    CourseStatsRow, FetchRecord, HorseStatsRow, JockeyStatsRow, Repository,
};
use paddock_use_case::{Error, Interactor, Result};

#[test]
fn source_key_and_url_follow_jra_layout() {
    let spec = MeetingSpec {
        year: 2026,
        round: 3,
        venue: Venue::Nakayama,
        day: 6,
    };
    assert_eq!(spec.source_key(), "2026-3-nakayama-6");
    assert_eq!(
        spec.pdf_url(),
        "https://www.jra.go.jp/datafile/seiseki/report/2026/2026-3nakayama6.pdf"
    );
}

// --- mocks ---------------------------------------------------------------

#[derive(Default)]
struct MockFetcher {
    /// Some(bytes) => 200, None => 404.
    body: Option<Vec<u8>>,
    calls: Mutex<usize>,
}

impl PdfFetcher for MockFetcher {
    fn fetch(&self, _url: &str) -> Result<Vec<u8>> {
        unimplemented!("fetch_meeting uses fetch_if_exists")
    }
    fn fetch_if_exists(&self, _url: &str) -> Result<Option<Vec<u8>>> {
        *self.calls.lock().unwrap() += 1;
        Ok(self.body.clone())
    }
}

struct OneRaceParser;

impl PdfParser for OneRaceParser {
    fn parse(&self, _bytes: &[u8]) -> Result<Vec<Race>> {
        Ok(vec![sample_race()])
    }
}

#[derive(Default)]
struct MockRepo {
    contains: bool,
    recorded: Mutex<Vec<FetchRecord>>,
    saved: Mutex<usize>,
}

impl Repository for MockRepo {
    async fn save_race(&self, _race: &Race) -> Result<()> {
        *self.saved.lock().unwrap() += 1;
        Ok(())
    }
    async fn horse_stats(&self, _name: &HorseName) -> Result<HorseStatsRow> {
        Err(Error::NotFound("unused".into()))
    }
    async fn course_stats(
        &self,
        _venue: Venue,
        _distance: u32,
        _surface: Surface,
    ) -> Result<CourseStatsRow> {
        Err(Error::NotFound("unused".into()))
    }
    async fn jockey_stats(&self, _name: &JockeyName) -> Result<JockeyStatsRow> {
        Err(Error::NotFound("unused".into()))
    }
    async fn count_races(&self) -> Result<u64> {
        Ok(0)
    }
    async fn race_exists(&self, _race_id: &RaceId) -> Result<bool> {
        Ok(false)
    }
    async fn fetch_history_contains(&self, _source_key: &str) -> Result<bool> {
        Ok(self.contains)
    }
    async fn record_fetch(&self, record: &FetchRecord) -> Result<()> {
        self.recorded.lock().unwrap().push(record.clone());
        Ok(())
    }
    async fn save_race_card(&self, _card: &RaceCard) -> Result<()> {
        Err(Error::NotFound("unused".into()))
    }
}

fn sample_race() -> Race {
    Race {
        race_id: RaceId::try_from("2026-3-nakayama-6-R1").unwrap(),
        date: NaiveDate::from_ymd_opt(2026, 4, 12).unwrap(),
        venue: Venue::Nakayama,
        round: 3,
        day: 6,
        race_num: 1,
        surface: Surface::Turf,
        distance: 2000,
        track_condition: None,
        weather: None,
        results: vec![HorseResult {
            finishing_position: Some(FinishingPosition::try_from(1u32).unwrap()),
            status: ResultStatus::Finished,
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(1u32).unwrap(),
            horse_name: HorseName::try_from("テストウマ").unwrap(),
            jockey: None,
            trainer: None,
            time_seconds: None,
            margin: None,
            odds: None,
            horse_weight: None,
            weight_change: None,
            weight_carried: None,
            popularity: None,
        }],
    }
}

fn spec() -> MeetingSpec {
    MeetingSpec {
        year: 2026,
        round: 3,
        venue: Venue::Nakayama,
        day: 6,
    }
}

#[tokio::test]
async fn ingests_and_records_history_when_new() {
    let interactor = Interactor::new(
        MockRepo::default(),
        OneRaceParser,
        MockFetcher {
            body: Some(vec![1, 2, 3]),
            ..Default::default()
        },
    );

    let resp = interactor.fetch_meeting(&spec(), false).await.unwrap();

    assert_eq!(
        resp.outcome,
        FetchMeetingOutcome::Ingested {
            races_saved: 1,
            horses_saved: 1,
        }
    );
    assert_eq!(*interactor.repository.saved.lock().unwrap(), 1);
    let recorded = interactor.repository.recorded.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].source_key, "2026-3-nakayama-6");
    // fetched_at is set by the use-case layer (not the gateway), so it is present
    // on the record handed to the repository.
    assert!(recorded[0].fetched_at <= chrono::Utc::now());
}

#[tokio::test]
async fn skips_without_fetching_when_already_in_history() {
    let interactor = Interactor::new(
        MockRepo {
            contains: true,
            ..Default::default()
        },
        OneRaceParser,
        MockFetcher {
            body: Some(vec![1]),
            ..Default::default()
        },
    );

    let resp = interactor.fetch_meeting(&spec(), false).await.unwrap();

    assert_eq!(resp.outcome, FetchMeetingOutcome::Skipped);
    assert_eq!(*interactor.pdf_fetcher.calls.lock().unwrap(), 0);
    assert_eq!(*interactor.repository.saved.lock().unwrap(), 0);
}

#[tokio::test]
async fn force_refetches_even_when_in_history() {
    let interactor = Interactor::new(
        MockRepo {
            contains: true,
            ..Default::default()
        },
        OneRaceParser,
        MockFetcher {
            body: Some(vec![1]),
            ..Default::default()
        },
    );

    let resp = interactor.fetch_meeting(&spec(), true).await.unwrap();

    assert!(matches!(resp.outcome, FetchMeetingOutcome::Ingested { .. }));
    assert_eq!(*interactor.pdf_fetcher.calls.lock().unwrap(), 1);
}

#[tokio::test]
async fn reports_not_found_and_records_nothing_on_404() {
    let interactor = Interactor::new(
        MockRepo::default(),
        OneRaceParser,
        MockFetcher {
            body: None,
            ..Default::default()
        },
    );

    let resp = interactor.fetch_meeting(&spec(), false).await.unwrap();

    assert_eq!(resp.outcome, FetchMeetingOutcome::NotFound);
    assert_eq!(*interactor.repository.saved.lock().unwrap(), 0);
    assert!(interactor.repository.recorded.lock().unwrap().is_empty());
}
