//! Offline tests for meeting-day discovery + exclusive control.
//!
//! JRA is unreachable from CI, so the fetcher/parser/repository are mocked.
//! These cover the URL/key derivation and the skip / not-found / ingest paths.

use std::sync::Mutex;

use chrono::NaiveDate;
use paddock_domain::horse_result::{FinishingPosition, GateNum, HorseName, HorseNum, ResultStatus};
use paddock_domain::{
    HorseResult, JockeyName, Race, RaceCard, RaceId, Surface, TrainerName, Venue,
};
use paddock_use_case::dto::pdf::fetch::{FetchMeetingOutcome, MeetingSpec};
use paddock_use_case::pdf_fetcher::PdfFetcher;
use paddock_use_case::pdf_parser::PdfParser;
use paddock_use_case::repository::{
    CourseStatsRow, FetchRecord, HorseStatsRow, JockeyStatsRow, Repository, TrainerStatsRow,
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
    async fn upsert_horse_history(
        &self,
        _horse_id: &paddock_domain::HorseId,
        runs: &[paddock_use_case::HorsePastRun],
    ) -> Result<usize> {
        Ok(runs.len())
    }
    async fn backfill_results_horse_ids(&self) -> Result<u64> {
        Ok(0)
    }
    async fn find_matching_horse_names(&self, _query: &str, _limit: u32) -> Result<Vec<String>> {
        unimplemented!()
    }
    async fn find_matching_jockey_names(&self, _query: &str, _limit: u32) -> Result<Vec<String>> {
        unimplemented!()
    }
    async fn find_matching_trainer_names(&self, _query: &str, _limit: u32) -> Result<Vec<String>> {
        unimplemented!()
    }
    async fn horse_stats(
        &self,
        _name: &HorseName,
        _as_of: Option<NaiveDate>,
    ) -> Result<HorseStatsRow> {
        Err(Error::NotFound("unused".into()))
    }
    async fn course_stats(
        &self,
        _venue: Venue,
        _distance: u32,
        _surface: Surface,
        _as_of: Option<NaiveDate>,
    ) -> Result<CourseStatsRow> {
        Err(Error::NotFound("unused".into()))
    }
    async fn jockey_stats(
        &self,
        _name: &JockeyName,
        _as_of: Option<NaiveDate>,
    ) -> Result<JockeyStatsRow> {
        Err(Error::NotFound("unused".into()))
    }
    async fn trainer_stats(
        &self,
        _name: &TrainerName,
        _as_of: Option<NaiveDate>,
    ) -> Result<TrainerStatsRow> {
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
    async fn save_race_odds(
        &self,
        _record: &paddock_use_case::repository::RaceOddsRecord,
    ) -> Result<()> {
        Err(Error::NotFound("unused".into()))
    }
    async fn find_race_card(&self, _race_id: &RaceId) -> Result<Option<RaceCard>> {
        Err(Error::NotFound("unused".into()))
    }

    async fn find_race_odds(
        &self,
        _race_id: &RaceId,
        _as_of: Option<NaiveDate>,
    ) -> Result<Option<paddock_domain::RaceOdds>> {
        Err(Error::NotFound("unused".into()))
    }

    async fn find_races_by_date(&self, _date: chrono::NaiveDate) -> Result<Vec<Race>> {
        Ok(Vec::new())
    }

    async fn find_finished_races_between(
        &self,
        _from: NaiveDate,
        _to: NaiveDate,
    ) -> Result<Vec<Race>> {
        Ok(Vec::new())
    }

    async fn find_recent_runs(
        &self,
        _name: &HorseName,
        _before: NaiveDate,
        _limit: u32,
    ) -> Result<Vec<(NaiveDate, HorseResult)>> {
        Ok(Vec::new())
    }

    async fn find_predict_session(
        &self,
        _date: chrono::NaiveDate,
    ) -> Result<Option<paddock_use_case::repository::PredictSessionRecord>> {
        Ok(None)
    }

    async fn find_predict_bets(
        &self,
        _date: chrono::NaiveDate,
    ) -> Result<Vec<paddock_use_case::repository::PredictBetRecord>> {
        Ok(Vec::new())
    }

    async fn save_predict_session(
        &self,
        _session: &paddock_use_case::repository::PredictSessionRecord,
    ) -> Result<()> {
        unimplemented!()
    }

    async fn save_race_outcome(
        &self,
        _session: &paddock_use_case::repository::PredictSessionRecord,
        _race_id: &RaceId,
        _bets: &[paddock_use_case::repository::PredictBetRecord],
    ) -> Result<()> {
        unimplemented!()
    }
    async fn find_predict_race_conditions(
        &self,
        _: chrono::NaiveDate,
    ) -> Result<Vec<paddock_use_case::repository::PredictRaceConditionRecord>> {
        unimplemented!()
    }
    async fn save_predict_race_condition(
        &self,
        _: chrono::NaiveDate,
        _: &paddock_use_case::repository::PredictRaceConditionRecord,
        _: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        unimplemented!()
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
    fn fetch_if_exists(&self, url: &str) -> Result<Option<Vec<u8>>> {
        if self.existing.contains(url) {
            Ok(Some(vec![1]))
        } else {
            Ok(None)
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
    fn fetch_if_exists(&self, url: &str) -> Result<Option<Vec<u8>>> {
        // URLs end with `...-1nakayama{day}.pdf`; error on configured days, else 404.
        let errors = self
            .error_days
            .iter()
            .any(|d| url.ends_with(&format!("1nakayama{d}.pdf")));
        if errors {
            Err(Error::Internal("simulated network failure".into()))
        } else {
            Ok(None)
        }
    }
}

/// Repository whose fetch-history is a fixed set of source keys.
#[derive(Default)]
struct HistoryRepo {
    history: HashSet<String>,
    saved: Mutex<usize>,
}

impl Repository for HistoryRepo {
    async fn save_race(&self, _race: &Race) -> Result<()> {
        *self.saved.lock().unwrap() += 1;
        Ok(())
    }
    async fn upsert_horse_history(
        &self,
        _horse_id: &paddock_domain::HorseId,
        runs: &[paddock_use_case::HorsePastRun],
    ) -> Result<usize> {
        Ok(runs.len())
    }
    async fn backfill_results_horse_ids(&self) -> Result<u64> {
        Ok(0)
    }
    async fn find_matching_horse_names(&self, _query: &str, _limit: u32) -> Result<Vec<String>> {
        unimplemented!()
    }
    async fn find_matching_jockey_names(&self, _query: &str, _limit: u32) -> Result<Vec<String>> {
        unimplemented!()
    }
    async fn find_matching_trainer_names(&self, _query: &str, _limit: u32) -> Result<Vec<String>> {
        unimplemented!()
    }
    async fn horse_stats(
        &self,
        _name: &HorseName,
        _as_of: Option<NaiveDate>,
    ) -> Result<HorseStatsRow> {
        Err(Error::NotFound("unused".into()))
    }
    async fn course_stats(
        &self,
        _venue: Venue,
        _distance: u32,
        _surface: Surface,
        _as_of: Option<NaiveDate>,
    ) -> Result<CourseStatsRow> {
        Err(Error::NotFound("unused".into()))
    }
    async fn jockey_stats(
        &self,
        _name: &JockeyName,
        _as_of: Option<NaiveDate>,
    ) -> Result<JockeyStatsRow> {
        Err(Error::NotFound("unused".into()))
    }
    async fn trainer_stats(
        &self,
        _name: &TrainerName,
        _as_of: Option<NaiveDate>,
    ) -> Result<TrainerStatsRow> {
        Err(Error::NotFound("unused".into()))
    }
    async fn count_races(&self) -> Result<u64> {
        Ok(0)
    }
    async fn race_exists(&self, _race_id: &RaceId) -> Result<bool> {
        Ok(false)
    }
    async fn fetch_history_contains(&self, source_key: &str) -> Result<bool> {
        Ok(self.history.contains(source_key))
    }
    async fn record_fetch(&self, _record: &FetchRecord) -> Result<()> {
        Ok(())
    }
    async fn save_race_card(&self, _card: &RaceCard) -> Result<()> {
        Err(Error::NotFound("unused".into()))
    }
    async fn save_race_odds(
        &self,
        _record: &paddock_use_case::repository::RaceOddsRecord,
    ) -> Result<()> {
        Err(Error::NotFound("unused".into()))
    }
    async fn find_race_card(&self, _race_id: &RaceId) -> Result<Option<RaceCard>> {
        Err(Error::NotFound("unused".into()))
    }

    async fn find_race_odds(
        &self,
        _race_id: &RaceId,
        _as_of: Option<NaiveDate>,
    ) -> Result<Option<paddock_domain::RaceOdds>> {
        Err(Error::NotFound("unused".into()))
    }

    async fn find_races_by_date(&self, _date: chrono::NaiveDate) -> Result<Vec<Race>> {
        Ok(Vec::new())
    }

    async fn find_finished_races_between(
        &self,
        _from: NaiveDate,
        _to: NaiveDate,
    ) -> Result<Vec<Race>> {
        Ok(Vec::new())
    }

    async fn find_recent_runs(
        &self,
        _name: &HorseName,
        _before: NaiveDate,
        _limit: u32,
    ) -> Result<Vec<(NaiveDate, HorseResult)>> {
        Ok(Vec::new())
    }

    async fn find_predict_session(
        &self,
        _date: chrono::NaiveDate,
    ) -> Result<Option<paddock_use_case::repository::PredictSessionRecord>> {
        Ok(None)
    }

    async fn find_predict_bets(
        &self,
        _date: chrono::NaiveDate,
    ) -> Result<Vec<paddock_use_case::repository::PredictBetRecord>> {
        Ok(Vec::new())
    }

    async fn save_predict_session(
        &self,
        _session: &paddock_use_case::repository::PredictSessionRecord,
    ) -> Result<()> {
        unimplemented!()
    }

    async fn save_race_outcome(
        &self,
        _session: &paddock_use_case::repository::PredictSessionRecord,
        _race_id: &RaceId,
        _bets: &[paddock_use_case::repository::PredictBetRecord],
    ) -> Result<()> {
        unimplemented!()
    }
    async fn find_predict_race_conditions(
        &self,
        _: chrono::NaiveDate,
    ) -> Result<Vec<paddock_use_case::repository::PredictRaceConditionRecord>> {
        unimplemented!()
    }
    async fn save_predict_race_condition(
        &self,
        _: chrono::NaiveDate,
        _: &paddock_use_case::repository::PredictRaceConditionRecord,
        _: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        unimplemented!()
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
        .fetch_meeting_range(&range, false, Duration::ZERO)
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
        .fetch_meeting_range(&range, false, Duration::ZERO)
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
        .fetch_meeting_range(&range, false, Duration::ZERO)
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
        .fetch_meeting_range(&range, true, Duration::ZERO)
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
        .fetch_meeting_range(&range, false, Duration::ZERO)
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
        .fetch_meeting_range(&range, false, Duration::ZERO)
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
