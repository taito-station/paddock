//! Offline tests for meeting-day discovery + exclusive control.
//!
//! JRA is unreachable from CI, so the fetcher/parser/repository are mocked.
//! These cover the URL/key derivation and the skip / not-found / ingest paths.

use std::sync::Mutex;

use chrono::NaiveDate;
use paddock_domain::horse_result::{FinishingPosition, GateNum, HorseName, HorseNum, ResultStatus};
use paddock_domain::{HorseResult, Race, RaceId, Surface, Venue};
use paddock_use_case::dto::pdf::fetch::{FetchMeetingOutcome, MeetingSpec};
use paddock_use_case::pdf_fetcher::{FetchProbe, PdfFetcher};
use paddock_use_case::pdf_parser::PdfParser;
use paddock_use_case::repository::{
    FetchDownload, FetchFailure, FetchRecord, FetchRepository, FetchStatus, RaceRepository,
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
    /// Some(bytes) => 200 (Found), None => 404 (Absent).
    body: Option<Vec<u8>>,
    calls: Mutex<usize>,
}

impl PdfFetcher for MockFetcher {
    fn fetch(&self, _url: &str) -> Result<Vec<u8>> {
        unimplemented!("fetch_meeting uses fetch_if_exists")
    }
    fn fetch_if_exists(&self, _url: &str) -> Result<FetchProbe> {
        *self.calls.lock().unwrap() += 1;
        Ok(match self.body.clone() {
            Some(bytes) => FetchProbe::Found(bytes),
            None => FetchProbe::Absent(404),
        })
    }
}

struct OneRaceParser;

impl PdfParser for OneRaceParser {
    fn parse(&self, _bytes: &[u8]) -> Result<Vec<Race>> {
        Ok(vec![sample_race()])
    }
}

/// Parser that yields no races, mirroring the 2025-autumn parser gap (issue #149):
/// the PDF is fetched fine but no race is extracted.
struct ZeroRaceParser;

impl PdfParser for ZeroRaceParser {
    fn parse(&self, _bytes: &[u8]) -> Result<Vec<Race>> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct MockRepo {
    contains: bool,
    /// Drives `fetch_status` (the Stage1 / download-only dedup check).
    status: Option<FetchStatus>,
    recorded: Mutex<Vec<FetchRecord>>,
    downloads: Mutex<Vec<FetchDownload>>,
    failures: Mutex<Vec<FetchFailure>>,
    saved: Mutex<usize>,
}

impl RaceRepository for MockRepo {
    async fn save_race(&self, _race: &Race) -> Result<()> {
        *self.saved.lock().unwrap() += 1;
        Ok(())
    }
    async fn count_races(&self) -> Result<u64> {
        Ok(0)
    }
    async fn race_exists(&self, _race_id: &RaceId) -> Result<bool> {
        Ok(false)
    }
    async fn find_races_by_date(&self, _date: chrono::NaiveDate) -> Result<Vec<Race>> {
        Ok(Vec::new())
    }
}

impl FetchRepository for MockRepo {
    async fn fetch_history_contains(&self, _source_key: &str) -> Result<bool> {
        Ok(self.contains)
    }
    async fn record_fetch(&self, record: &FetchRecord) -> Result<()> {
        self.recorded.lock().unwrap().push(record.clone());
        Ok(())
    }
    async fn fetch_status(&self, _source_key: &str) -> Result<Option<FetchStatus>> {
        Ok(self.status)
    }
    async fn record_download(&self, record: &FetchDownload) -> Result<()> {
        self.downloads.lock().unwrap().push(record.clone());
        Ok(())
    }
    async fn record_failure(&self, record: &FetchFailure) -> Result<()> {
        self.failures.lock().unwrap().push(record.clone());
        Ok(())
    }
}

/// Parser that panics if invoked — used to prove Stage1 (`--download-only`) never
/// parses the fetched PDF.
struct PanicParser;

impl PdfParser for PanicParser {
    fn parse(&self, _bytes: &[u8]) -> Result<Vec<Race>> {
        panic!("download-only must not parse the PDF");
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
            horse_id: None,
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

    let resp = interactor
        .fetch_meeting(&spec(), false, None)
        .await
        .unwrap();

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

    let resp = interactor
        .fetch_meeting(&spec(), false, None)
        .await
        .unwrap();

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

    let resp = interactor.fetch_meeting(&spec(), true, None).await.unwrap();

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

    let resp = interactor
        .fetch_meeting(&spec(), false, None)
        .await
        .unwrap();

    assert_eq!(resp.outcome, FetchMeetingOutcome::NotFound { http_status: 404 });
    assert_eq!(*interactor.repository.saved.lock().unwrap(), 0);
    assert!(interactor.repository.recorded.lock().unwrap().is_empty());
    // A single fetch_meeting never records a failure (no adjacency knowledge).
    assert!(interactor.repository.failures.lock().unwrap().is_empty());
}

#[tokio::test]
async fn reports_empty_and_records_nothing_when_zero_races_parsed() {
    // The PDF is fetched (Some bytes) but the parser extracts no race. The
    // meeting must NOT be recorded in history, so a later run can re-fetch it
    // instead of being self-blocked as a "successful" 0-race ingest (#149).
    let interactor = Interactor::new(
        MockRepo::default(),
        ZeroRaceParser,
        MockFetcher {
            body: Some(vec![1, 2, 3]),
            ..Default::default()
        },
    );

    let resp = interactor
        .fetch_meeting(&spec(), false, None)
        .await
        .unwrap();

    assert_eq!(resp.outcome, FetchMeetingOutcome::Empty);
    assert_eq!(*interactor.repository.saved.lock().unwrap(), 0);
    assert!(
        interactor.repository.recorded.lock().unwrap().is_empty(),
        "a 0-race parse must not be recorded in fetch history"
    );
}

// --- stage 1: download-only ----------------------------------------------

#[tokio::test]
async fn download_only_writes_inbox_and_records_without_parsing() {
    let inbox = tempfile::tempdir().unwrap();
    let interactor = Interactor::new(
        MockRepo::default(),
        // PanicParser proves the PDF is never parsed in Stage1.
        PanicParser,
        MockFetcher {
            body: Some(vec![9, 8, 7]),
            ..Default::default()
        },
    );

    let resp = interactor
        .fetch_meeting(&spec(), false, Some(inbox.path()))
        .await
        .unwrap();

    let expected = inbox.path().join("2026-3nakayama6.pdf");
    assert_eq!(
        resp.outcome,
        FetchMeetingOutcome::Downloaded {
            path: expected.clone()
        }
    );
    // The raw PDF is written verbatim to the inbox.
    assert_eq!(std::fs::read(&expected).unwrap(), vec![9, 8, 7]);
    // Recorded as downloaded (Stage1), not ingested; no race was saved.
    let downloads = interactor.repository.downloads.lock().unwrap();
    assert_eq!(downloads.len(), 1);
    assert_eq!(downloads[0].source_key, "2026-3-nakayama-6");
    assert!(interactor.repository.recorded.lock().unwrap().is_empty());
    assert_eq!(*interactor.repository.saved.lock().unwrap(), 0);
}

#[tokio::test]
async fn download_only_skips_when_already_downloaded_or_ingested() {
    for status in [FetchStatus::Downloaded, FetchStatus::Ingested] {
        let inbox = tempfile::tempdir().unwrap();
        let interactor = Interactor::new(
            MockRepo {
                status: Some(status),
                ..Default::default()
            },
            PanicParser,
            MockFetcher {
                body: Some(vec![1]),
                ..Default::default()
            },
        );

        let resp = interactor
            .fetch_meeting(&spec(), false, Some(inbox.path()))
            .await
            .unwrap();

        assert_eq!(resp.outcome, FetchMeetingOutcome::Skipped);
        // No network hit and nothing written when already in the lifecycle.
        assert_eq!(*interactor.pdf_fetcher.calls.lock().unwrap(), 0);
        assert!(interactor.repository.downloads.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn download_only_reports_not_found_and_writes_nothing_on_404() {
    let inbox = tempfile::tempdir().unwrap();
    let interactor = Interactor::new(
        MockRepo::default(),
        PanicParser,
        MockFetcher {
            body: None, // 404
            ..Default::default()
        },
    );

    let resp = interactor
        .fetch_meeting(&spec(), false, Some(inbox.path()))
        .await
        .unwrap();

    assert_eq!(resp.outcome, FetchMeetingOutcome::NotFound { http_status: 404 });
    assert!(interactor.repository.downloads.lock().unwrap().is_empty());
    assert_eq!(std::fs::read_dir(inbox.path()).unwrap().count(), 0);
}

// --- stage 2: ingest from inbox ------------------------------------------

#[tokio::test]
async fn ingesting_an_inbox_meeting_records_ingested_and_removes_the_pdf() {
    // A Stage1 download leaves `<inbox>/2026-3nakayama6.pdf`; Stage2 ingest parses
    // it, records the meeting as ingested (single source of truth), and deletes it.
    let dir = tempfile::tempdir().unwrap();
    let inbox = dir.path().join("inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    let pdf = inbox.join("2026-3nakayama6.pdf");
    std::fs::write(&pdf, vec![1, 2, 3]).unwrap();

    let interactor = Interactor::new(MockRepo::default(), OneRaceParser, MockFetcher::default());

    let resp = interactor.ingest_pdf(pdf.to_str().unwrap()).await.unwrap();

    assert_eq!(resp.races_saved, 1);
    assert_eq!(*interactor.repository.saved.lock().unwrap(), 1);
    let recorded = interactor.repository.recorded.lock().unwrap();
    assert_eq!(recorded.len(), 1, "Stage2 records the meeting ingested");
    assert_eq!(recorded[0].source_key, "2026-3-nakayama-6");
    assert!(!pdf.exists(), "the inbox PDF is removed after ingest");
}

#[tokio::test]
async fn ingesting_a_zero_race_inbox_pdf_keeps_it_and_records_nothing() {
    // A parser gap (0 races) must not be recorded and the PDF must stay so it can
    // be re-ingested once the parser is fixed (#149).
    let dir = tempfile::tempdir().unwrap();
    let inbox = dir.path().join("inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    let pdf = inbox.join("2026-3nakayama6.pdf");
    std::fs::write(&pdf, vec![1, 2, 3]).unwrap();

    let interactor = Interactor::new(MockRepo::default(), ZeroRaceParser, MockFetcher::default());

    let resp = interactor.ingest_pdf(pdf.to_str().unwrap()).await.unwrap();

    assert_eq!(resp.races_saved, 0);
    assert!(interactor.repository.recorded.lock().unwrap().is_empty());
    assert!(pdf.exists(), "a 0-race PDF is kept for re-ingest");
}

// --- range fetch ---------------------------------------------------------

use std::collections::HashSet;
use std::time::Duration;

use paddock_use_case::dto::pdf::fetch::MeetingRange;

/// Fetcher backed by a fixed set of "existing" URLs: 200 for members, 404 otherwise.
struct ExistingUrlsFetcher {
    existing: HashSet<String>,
}

impl PdfFetcher for ExistingUrlsFetcher {
    fn fetch(&self, _url: &str) -> Result<Vec<u8>> {
        unimplemented!("range fetch uses fetch_if_exists")
    }
    fn fetch_if_exists(&self, url: &str) -> Result<FetchProbe> {
        if self.existing.contains(url) {
            Ok(FetchProbe::Found(vec![1]))
        } else {
            Ok(FetchProbe::Absent(404))
        }
    }
}

/// Fetcher: 200 for `existing` URLs, else returns the configured absent status.
/// Used to exercise a 403 boundary (a possibly-transient JRA block), not just 404.
struct AbsentStatusFetcher {
    existing: HashSet<String>,
    absent_status: u16,
}

impl PdfFetcher for AbsentStatusFetcher {
    fn fetch(&self, _url: &str) -> Result<Vec<u8>> {
        unimplemented!("range fetch uses fetch_if_exists")
    }
    fn fetch_if_exists(&self, url: &str) -> Result<FetchProbe> {
        if self.existing.contains(url) {
            Ok(FetchProbe::Found(vec![1]))
        } else {
            Ok(FetchProbe::Absent(self.absent_status))
        }
    }
}

/// Fetcher whose `fetch_if_exists` always errors (e.g. a network failure), used to verify
/// that range fetch counts failures and keeps going rather than aborting.
struct ErrorOnDayFetcher {
    /// Day numbers (within round 1) that should error; others 404.
    error_days: HashSet<u32>,
}

impl PdfFetcher for ErrorOnDayFetcher {
    fn fetch(&self, _url: &str) -> Result<Vec<u8>> {
        unimplemented!("range fetch uses fetch_if_exists")
    }
    fn fetch_if_exists(&self, url: &str) -> Result<FetchProbe> {
        // URLs end with `...-1nakayama{day}.pdf`; error on configured days, else 404.
        let errors = self
            .error_days
            .iter()
            .any(|d| url.ends_with(&format!("1nakayama{d}.pdf")));
        if errors {
            Err(Error::Internal("simulated network failure".into()))
        } else {
            Ok(FetchProbe::Absent(404))
        }
    }
}

/// Repository whose fetch-history is a fixed set of source keys.
#[derive(Default)]
struct HistoryRepo {
    history: HashSet<String>,
    saved: Mutex<usize>,
    /// Failures persisted via `record_failure` (the boundary 403/404 retry rows).
    failures: Mutex<Vec<FetchFailure>>,
}

impl RaceRepository for HistoryRepo {
    async fn save_race(&self, _race: &Race) -> Result<()> {
        *self.saved.lock().unwrap() += 1;
        Ok(())
    }
    async fn count_races(&self) -> Result<u64> {
        Ok(0)
    }
    async fn race_exists(&self, _race_id: &RaceId) -> Result<bool> {
        Ok(false)
    }
    async fn find_races_by_date(&self, _date: chrono::NaiveDate) -> Result<Vec<Race>> {
        Ok(Vec::new())
    }
}

impl FetchRepository for HistoryRepo {
    async fn fetch_history_contains(&self, source_key: &str) -> Result<bool> {
        Ok(self.history.contains(source_key))
    }
    async fn record_fetch(&self, _record: &FetchRecord) -> Result<()> {
        Ok(())
    }
    async fn fetch_status(&self, source_key: &str) -> Result<Option<FetchStatus>> {
        Ok(self
            .history
            .contains(source_key)
            .then_some(FetchStatus::Ingested))
    }
    async fn record_download(&self, _record: &FetchDownload) -> Result<()> {
        Ok(())
    }
    async fn record_failure(&self, record: &FetchFailure) -> Result<()> {
        self.failures.lock().unwrap().push(record.clone());
        Ok(())
    }
}

fn url_for(year: i32, round: u32, venue: Venue, day: u32) -> String {
    paddock_use_case::dto::pdf::fetch::MeetingSpec {
        year,
        round,
        venue,
        day,
    }
    .pdf_url()
}

#[tokio::test]
async fn round_wide_stops_at_first_missing_day() {
    // Nakayama, round 3: days 1-3 exist, day 4 is 404.
    let existing: HashSet<String> = (1..=3)
        .map(|d| url_for(2026, 3, Venue::Nakayama, d))
        .collect();
    let interactor = Interactor::new(
        HistoryRepo::default(),
        OneRaceParser,
        ExistingUrlsFetcher { existing },
    );

    let range = MeetingRange {
        year: 2026,
        venue: Some(Venue::Nakayama),
        round: Some(3),
        day: None,
    };
    let summary = interactor
        .fetch_meeting_range(&range, false, Duration::ZERO, None)
        .await
        .unwrap();

    assert_eq!(summary.ingested, 3);
    assert_eq!(summary.not_found, 1); // day 4
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.races_saved, 3);
}

#[tokio::test]
async fn year_wide_enumerates_every_venue() {
    // Only Nakayama races: round 1 days 1-2 exist; everything else 404.
    let existing: HashSet<String> = (1..=2)
        .map(|d| url_for(2026, 1, Venue::Nakayama, d))
        .collect();
    let interactor = Interactor::new(
        HistoryRepo::default(),
        OneRaceParser,
        ExistingUrlsFetcher { existing },
    );

    let range = MeetingRange {
        year: 2026,
        venue: None,
        round: None,
        day: None,
    };
    let summary = interactor
        .fetch_meeting_range(&range, false, Duration::ZERO, None)
        .await
        .unwrap();

    assert_eq!(summary.ingested, 2);
    // The other 9 venues are each probed once (round 1 day 1 → 404, stop).
    assert!(
        summary.not_found >= 9,
        "expected every venue probed, got not_found={}",
        summary.not_found
    );
    assert_eq!(summary.failed, 0);
}

#[tokio::test]
async fn already_ingested_days_are_skipped_and_enumeration_continues() {
    // Days 1-3 are in history (so skipped without fetching); day 4 does not exist.
    let history: HashSet<String> = (1..=3)
        .map(|d| {
            paddock_use_case::dto::pdf::fetch::MeetingSpec {
                year: 2026,
                round: 3,
                venue: Venue::Nakayama,
                day: d,
            }
            .source_key()
        })
        .collect();
    let interactor = Interactor::new(
        HistoryRepo {
            history,
            ..Default::default()
        },
        OneRaceParser,
        ExistingUrlsFetcher {
            existing: HashSet::new(), // nothing fetchable; day 4 → 404
        },
    );

    let range = MeetingRange {
        year: 2026,
        venue: Some(Venue::Nakayama),
        round: Some(3),
        day: None,
    };
    let summary = interactor
        .fetch_meeting_range(&range, false, Duration::ZERO, None)
        .await
        .unwrap();

    assert_eq!(summary.skipped, 3);
    assert_eq!(summary.ingested, 0);
    assert_eq!(summary.not_found, 1); // day 4 stops the loop
}

#[tokio::test]
async fn force_refetches_history_entries() {
    let history: HashSet<String> = (1..=3)
        .map(|d| {
            paddock_use_case::dto::pdf::fetch::MeetingSpec {
                year: 2026,
                round: 3,
                venue: Venue::Nakayama,
                day: d,
            }
            .source_key()
        })
        .collect();
    let existing: HashSet<String> = (1..=3)
        .map(|d| url_for(2026, 3, Venue::Nakayama, d))
        .collect();
    let interactor = Interactor::new(
        HistoryRepo {
            history,
            ..Default::default()
        },
        OneRaceParser,
        ExistingUrlsFetcher { existing },
    );

    let range = MeetingRange {
        year: 2026,
        venue: Some(Venue::Nakayama),
        round: Some(3),
        day: None,
    };
    let summary = interactor
        .fetch_meeting_range(&range, true, Duration::ZERO, None)
        .await
        .unwrap();

    assert_eq!(summary.ingested, 3); // force re-fetches despite history
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.not_found, 1);
}

#[tokio::test]
async fn fully_specified_range_is_one_meeting() {
    let existing: HashSet<String> = [url_for(2026, 3, Venue::Nakayama, 6)].into_iter().collect();
    let interactor = Interactor::new(
        HistoryRepo::default(),
        OneRaceParser,
        ExistingUrlsFetcher { existing },
    );

    let range = MeetingRange {
        year: 2026,
        venue: Some(Venue::Nakayama),
        round: Some(3),
        day: Some(6),
    };
    let summary = interactor
        .fetch_meeting_range(&range, false, Duration::ZERO, None)
        .await
        .unwrap();

    assert_eq!(summary.ingested, 1);
    assert_eq!(summary.not_found, 0);
    assert_eq!(summary.skipped, 0);
}

#[tokio::test]
async fn errors_are_counted_and_do_not_abort_the_range() {
    // Round 1, days 1-2 error (network failure); day 3 is 404 and stops the round.
    let interactor = Interactor::new(
        HistoryRepo::default(),
        OneRaceParser,
        ErrorOnDayFetcher {
            error_days: [1, 2].into_iter().collect(),
        },
    );

    let range = MeetingRange {
        year: 2026,
        venue: Some(Venue::Nakayama),
        round: Some(1),
        day: None,
    };
    let summary = interactor
        .fetch_meeting_range(&range, false, Duration::ZERO, None)
        .await
        .unwrap();

    assert_eq!(summary.failed, 2, "both erroring days are counted");
    assert_eq!(summary.failures.len(), 2);
    assert_eq!(summary.ingested, 0);
    assert_eq!(summary.not_found, 1); // day 3 stops the round
    // Enumeration continued past the errors to find the 404 boundary.
    assert!(
        summary
            .failures
            .iter()
            .all(|(key, _)| key.contains("nakayama")),
        "failures carry the meeting source_key"
    );
}

#[tokio::test]
async fn empty_meetings_are_counted_and_enumeration_continues() {
    // Nakayama round 3: days 1-3 exist but every PDF parses to 0 races; day 4 is
    // 404. The empty days must be counted (not recorded) and must not stop the
    // day loop — only the 404 boundary does.
    let existing: HashSet<String> = (1..=3)
        .map(|d| url_for(2026, 3, Venue::Nakayama, d))
        .collect();
    let interactor = Interactor::new(
        HistoryRepo::default(),
        ZeroRaceParser,
        ExistingUrlsFetcher { existing },
    );

    let range = MeetingRange {
        year: 2026,
        venue: Some(Venue::Nakayama),
        round: Some(3),
        day: None,
    };
    let summary = interactor
        .fetch_meeting_range(&range, false, Duration::ZERO, None)
        .await
        .unwrap();

    assert_eq!(summary.empty, 3);
    assert_eq!(summary.ingested, 0);
    assert_eq!(summary.not_found, 1); // day 4 stops the round
    assert_eq!(summary.failed, 0);
    assert_eq!(*interactor.repository.saved.lock().unwrap(), 0);
}

#[tokio::test]
async fn empty_day1_does_not_stop_round_enumeration() {
    // round unspecified: round 1 and round 2 each have a day-1 PDF that parses
    // to 0 races (Empty); day 2 of each is 404; round 3 day 1 is 404. An Empty
    // day 1 must NOT be mistaken for an absent round (only a 404 day 1 is), so
    // enumeration must reach round 2 — proven by empty == 2.
    let existing: HashSet<String> = [
        url_for(2026, 1, Venue::Nakayama, 1),
        url_for(2026, 2, Venue::Nakayama, 1),
    ]
    .into_iter()
    .collect();
    let interactor = Interactor::new(
        HistoryRepo::default(),
        ZeroRaceParser,
        ExistingUrlsFetcher { existing },
    );

    let range = MeetingRange {
        year: 2026,
        venue: Some(Venue::Nakayama),
        round: None,
        day: None,
    };
    let summary = interactor
        .fetch_meeting_range(&range, false, Duration::ZERO, None)
        .await
        .unwrap();

    assert_eq!(
        summary.empty, 2,
        "round 2 was reached despite round 1 day 1 empty"
    );
    assert_eq!(summary.ingested, 0);
    assert_eq!(summary.failed, 0);
}

// --- #170: boundary 403/404 recorded as retryable `failed` -----------------

#[tokio::test]
async fn boundary_absence_after_successes_is_recorded_as_failed() {
    // Nakayama round 3: days 1-3 exist, day 4 is 404. Day 4 follows confirmed
    // successes, so it is the "連続成功直後の単発404" boundary — a plausibly-real
    // day (not-yet-published or transient block) persisted as a retryable failed row.
    let existing: HashSet<String> = (1..=3)
        .map(|d| url_for(2026, 3, Venue::Nakayama, d))
        .collect();
    let interactor = Interactor::new(
        HistoryRepo::default(),
        OneRaceParser,
        ExistingUrlsFetcher { existing },
    );

    let range = MeetingRange {
        year: 2026,
        venue: Some(Venue::Nakayama),
        round: Some(3),
        day: None,
    };
    let summary = interactor
        .fetch_meeting_range(&range, false, Duration::ZERO, None)
        .await
        .unwrap();

    assert_eq!(summary.ingested, 3);
    assert_eq!(summary.not_found, 1); // day 4
    assert_eq!(summary.recorded_failed, 1, "the boundary day 4 is recorded");
    let failures = interactor.repository.failures.lock().unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].source_key, "2026-3-nakayama-4");
    assert_eq!(failures[0].http_status, 404);
}

#[tokio::test]
async fn boundary_403_after_successes_records_failed_with_403_status() {
    // A 403 at the boundary (not just 404) is the case ADR0024 論点1 cares about:
    // JRA returns 403 on a transient block of a real meeting. Days 1-2 exist, day 3
    // is 403 → recorded as failed carrying its 403 status (not flattened to 404).
    let existing: HashSet<String> = (1..=2)
        .map(|d| url_for(2026, 3, Venue::Nakayama, d))
        .collect();
    let interactor = Interactor::new(
        HistoryRepo::default(),
        OneRaceParser,
        AbsentStatusFetcher {
            existing,
            absent_status: 403,
        },
    );

    let range = MeetingRange {
        year: 2026,
        venue: Some(Venue::Nakayama),
        round: Some(3),
        day: None,
    };
    let summary = interactor
        .fetch_meeting_range(&range, false, Duration::ZERO, None)
        .await
        .unwrap();

    assert_eq!(summary.ingested, 2);
    assert_eq!(summary.recorded_failed, 1);
    let failures = interactor.repository.failures.lock().unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].source_key, "2026-3-nakayama-3");
    assert_eq!(failures[0].http_status, 403, "403 が 404 に潰れず保持される");
}

#[tokio::test]
async fn absent_round_day1_is_not_recorded_as_junk() {
    // round unspecified. Round 1 days 1-2 exist (day 3 absent = boundary after
    // success → recorded); round 2 day 1 absent = the round does not exist → the
    // round-nonexistence boundary, which must NOT be recorded (grid junk). This is
    // the core #170 distinction: a real boundary is kept, a non-existent round dropped.
    let existing: HashSet<String> = (1..=2)
        .map(|d| url_for(2026, 1, Venue::Nakayama, d))
        .collect();
    let interactor = Interactor::new(
        HistoryRepo::default(),
        OneRaceParser,
        ExistingUrlsFetcher { existing },
    );

    let range = MeetingRange {
        year: 2026,
        venue: Some(Venue::Nakayama),
        round: None,
        day: None,
    };
    let summary = interactor
        .fetch_meeting_range(&range, false, Duration::ZERO, None)
        .await
        .unwrap();

    assert_eq!(summary.ingested, 2);
    assert_eq!(summary.recorded_failed, 1, "only the real boundary is recorded");
    let failures = interactor.repository.failures.lock().unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].source_key, "2026-1-nakayama-3");
    // Round 2 day 1 (the round-nonexistence boundary) is junk — never recorded.
    assert!(
        !failures.iter().any(|f| f.source_key == "2026-2-nakayama-1"),
        "a day-1-absent round must not produce a failed row"
    );
}

#[tokio::test]
async fn download_only_does_not_skip_a_failed_row() {
    // A `failed` row is a re-fetch candidate, not a skip: Stage1 dedup matches only
    // Downloaded/Ingested, so a Failed status re-fetches (永久スキップ防止, #170).
    let inbox = tempfile::tempdir().unwrap();
    let interactor = Interactor::new(
        MockRepo {
            status: Some(FetchStatus::Failed),
            ..Default::default()
        },
        PanicParser, // Stage1 must not parse
        MockFetcher {
            body: Some(vec![1, 2, 3]),
            ..Default::default()
        },
    );

    let resp = interactor
        .fetch_meeting(&spec(), false, Some(inbox.path()))
        .await
        .unwrap();

    assert!(
        matches!(resp.outcome, FetchMeetingOutcome::Downloaded { .. }),
        "a failed row must be re-fetched, not skipped: {:?}",
        resp.outcome
    );
    assert_eq!(*interactor.pdf_fetcher.calls.lock().unwrap(), 1);
    assert_eq!(interactor.repository.downloads.lock().unwrap().len(), 1);
}
