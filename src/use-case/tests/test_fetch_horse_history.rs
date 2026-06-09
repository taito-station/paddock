//! Offline tests for the netkeiba horse-history ingest interactor.
//!
//! netkeiba is unreachable from CI, so the scraper and repository are mocked.
//! These cover the core flow: each target horse's runs are upserted per-horse
//! (no cross-horse synthetic-race merge), per-horse fetch failures are skipped,
//! and duplicate netkeiba_race_id rows for one horse are de-duplicated.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use chrono::NaiveDate;
use paddock_domain::horse_result::{FinishingPosition, GateNum, HorseName, HorseNum, ResultStatus};
use paddock_domain::{HorseId, HorseResult, JockeyName, Race, RaceCard, RaceId, Surface, Venue};
use paddock_use_case::netkeiba_scraper::{HorsePastRun, NetkeibaScraper, RunnerRef};
use paddock_use_case::repository::{
    CourseStatsRow, FetchRecord, HorseStatsRow, JockeyStatsRow, PredictBetRecord,
    PredictSessionRecord, Repository,
};
use paddock_use_case::{Error, HorseHistoryInteractor, Result};

// --- builders ------------------------------------------------------------

fn past_run(nk_race: &str, horse: &str, horse_num: u32, finish: u32) -> HorsePastRun {
    HorsePastRun {
        netkeiba_race_id: nk_race.to_string(),
        date: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
        venue: Venue::Tokyo,
        round: 1,
        day: 1,
        race_num: 11,
        surface: Surface::Turf,
        distance: 1600,
        track_condition: None,
        finishing_position: Some(FinishingPosition::try_from(finish).unwrap()),
        status: ResultStatus::Finished,
        gate_num: GateNum::try_from(1u32).unwrap(),
        horse_num: HorseNum::try_from(horse_num).unwrap(),
        horse_name: HorseName::try_from(horse).unwrap(),
        jockey: None,
        time_seconds: None,
        margin: None,
        odds: None,
        horse_weight: None,
        weight_change: None,
        weight_carried: None,
        popularity: None,
    }
}

// --- mocks ---------------------------------------------------------------

struct FakeScraper {
    /// shutuba race_id -> その出走馬の horse_id 群
    shutuba: HashMap<String, Vec<String>>,
    /// horse_id -> 近走
    histories: HashMap<String, Vec<HorsePastRun>>,
    /// fetch_horse_history が失敗する horse_id
    failing: HashSet<String>,
}

impl NetkeibaScraper for FakeScraper {
    fn fetch_shutuba(&self, race_id: &str) -> Result<Vec<RunnerRef>> {
        let ids = self
            .shutuba
            .get(race_id)
            .ok_or_else(|| Error::NotFound(race_id.into()))?;
        Ok(ids
            .iter()
            .enumerate()
            .map(|(i, id)| RunnerRef {
                horse_num: HorseNum::try_from((i + 1) as u32).unwrap(),
                horse_name: HorseName::try_from(format!("出走馬{}", i + 1)).unwrap(),
                horse_id: HorseId::try_from(id.clone()).unwrap(),
            })
            .collect())
    }

    fn fetch_horse_history(&self, horse_id: &HorseId) -> Result<Vec<HorsePastRun>> {
        if self.failing.contains(horse_id.value()) {
            return Err(Error::Internal("boom".into()));
        }
        Ok(self
            .histories
            .get(horse_id.value())
            .cloned()
            .unwrap_or_default())
    }

    fn fetch_card(
        &self,
        _race_id: &str,
    ) -> Result<paddock_use_case::netkeiba_scraper::FetchedCard> {
        unimplemented!()
    }

    fn fetch_win_odds(
        &self,
        _race_id: &str,
    ) -> Result<Vec<paddock_use_case::netkeiba_scraper::FetchedWinOdds>> {
        unimplemented!()
    }
}

#[derive(Default)]
struct RecordingRepo {
    /// upsert_horse_history の呼び出しを (horse_id, runs) で記録する。
    upserted: Mutex<Vec<(String, Vec<HorsePastRun>)>>,
}

impl Repository for RecordingRepo {
    async fn upsert_horse_history(&self, horse_id: &HorseId, runs: &[HorsePastRun]) -> Result<()> {
        self.upserted
            .lock()
            .unwrap()
            .push((horse_id.value().to_string(), runs.to_vec()));
        Ok(())
    }
    async fn backfill_results_horse_ids(&self) -> Result<u64> {
        Ok(0)
    }
    async fn save_race(&self, _race: &Race) -> Result<()> {
        unimplemented!()
    }
    async fn horse_stats(
        &self,
        _name: &HorseName,
        _as_of: Option<NaiveDate>,
    ) -> Result<HorseStatsRow> {
        unimplemented!()
    }
    async fn course_stats(
        &self,
        _venue: Venue,
        _distance: u32,
        _surface: Surface,
        _as_of: Option<NaiveDate>,
    ) -> Result<CourseStatsRow> {
        unimplemented!()
    }
    async fn jockey_stats(
        &self,
        _name: &JockeyName,
        _as_of: Option<NaiveDate>,
    ) -> Result<JockeyStatsRow> {
        unimplemented!()
    }
    async fn count_races(&self) -> Result<u64> {
        unimplemented!()
    }
    async fn race_exists(&self, _race_id: &RaceId) -> Result<bool> {
        unimplemented!()
    }
    async fn fetch_history_contains(&self, _source_key: &str) -> Result<bool> {
        unimplemented!()
    }
    async fn record_fetch(&self, _record: &FetchRecord) -> Result<()> {
        unimplemented!()
    }
    async fn save_race_card(&self, _card: &RaceCard) -> Result<()> {
        unimplemented!()
    }
    async fn save_race_odds(
        &self,
        _record: &paddock_use_case::repository::RaceOddsRecord,
    ) -> Result<()> {
        unimplemented!()
    }
    async fn find_race_card(&self, _race_id: &RaceId) -> Result<Option<RaceCard>> {
        unimplemented!()
    }
    async fn find_races_by_date(&self, _date: NaiveDate) -> Result<Vec<Race>> {
        unimplemented!()
    }
    async fn find_finished_races_between(
        &self,
        _from: NaiveDate,
        _to: NaiveDate,
    ) -> Result<Vec<Race>> {
        unimplemented!()
    }
    async fn find_recent_runs(
        &self,
        _name: &HorseName,
        _before: NaiveDate,
        _limit: u32,
    ) -> Result<Vec<(NaiveDate, HorseResult)>> {
        unimplemented!()
    }
    async fn find_predict_session(&self, _date: NaiveDate) -> Result<Option<PredictSessionRecord>> {
        unimplemented!()
    }
    async fn find_predict_bets(&self, _date: NaiveDate) -> Result<Vec<PredictBetRecord>> {
        unimplemented!()
    }
    async fn save_predict_session(&self, _session: &PredictSessionRecord) -> Result<()> {
        unimplemented!()
    }
    async fn save_race_outcome(
        &self,
        _session: &PredictSessionRecord,
        _race_id: &RaceId,
        _bets: &[PredictBetRecord],
    ) -> Result<()> {
        unimplemented!()
    }
}

// --- tests ---------------------------------------------------------------

#[tokio::test]
async fn stores_each_horse_runs_separately() {
    // 出馬表 R に H1,H2 が出走。両馬は過去レース rA を共に走り、固有レース(rB,rC)も持つ。
    // 旧実装の「合成レースへの馬横断集約」は廃止し、各馬の近走を別々に保存する。
    let scraper = FakeScraper {
        shutuba: HashMap::from([("R".to_string(), vec!["H1".to_string(), "H2".to_string()])]),
        histories: HashMap::from([
            (
                "H1".to_string(),
                vec![
                    past_run("rA", "ウマエー", 3, 2),
                    past_run("rB", "ウマエー", 1, 1),
                ],
            ),
            (
                "H2".to_string(),
                vec![
                    past_run("rA", "ウマビー", 5, 4),
                    past_run("rC", "ウマビー", 1, 6),
                ],
            ),
        ]),
        failing: HashSet::new(),
    };
    let interactor = HorseHistoryInteractor::new(RecordingRepo::default(), scraper);

    let resp = interactor
        .fetch_and_store(&["R".to_string()], &[])
        .await
        .expect("fetch_and_store");

    assert_eq!(resp.horses_fetched, 2);
    assert_eq!(resp.horses_failed, 0);
    assert_eq!(resp.runs_saved, 4, "H1:2 + H2:2 走");

    let calls = interactor.repository.upserted.lock().unwrap();
    assert_eq!(calls.len(), 2, "馬ごとに 1 回 upsert");
    let by_horse: HashMap<&str, usize> = calls
        .iter()
        .map(|(id, runs)| (id.as_str(), runs.len()))
        .collect();
    assert_eq!(by_horse.get("H1"), Some(&2));
    assert_eq!(by_horse.get("H2"), Some(&2));
}

#[tokio::test]
async fn skips_failing_horse_and_continues() {
    let scraper = FakeScraper {
        shutuba: HashMap::new(),
        histories: HashMap::from([("OK".to_string(), vec![past_run("rX", "ウマ", 1, 1)])]),
        failing: HashSet::from(["BAD".to_string()]),
    };
    let interactor = HorseHistoryInteractor::new(RecordingRepo::default(), scraper);

    // 直接 horse_id 指定（出馬表バイパス）。BAD は失敗しスキップ、OK は取り込む。
    let resp = interactor
        .fetch_and_store(&[], &["BAD".to_string(), "OK".to_string()])
        .await
        .expect("fetch_and_store");

    assert_eq!(resp.horses_fetched, 1);
    assert_eq!(resp.horses_failed, 1);
    assert_eq!(resp.runs_saved, 1);
}

#[tokio::test]
async fn dedups_same_netkeiba_race() {
    // 異常 HTML 等で同一馬が同一過去レースの行を 2 つ返しても 1 走に集約する。
    let scraper = FakeScraper {
        shutuba: HashMap::new(),
        histories: HashMap::from([(
            "DUP".to_string(),
            vec![
                past_run("rDup", "ウマ", 4, 1),
                past_run("rDup", "ウマ", 4, 1),
            ],
        )]),
        failing: HashSet::new(),
    };
    let interactor = HorseHistoryInteractor::new(RecordingRepo::default(), scraper);

    let resp = interactor
        .fetch_and_store(&[], &["DUP".to_string()])
        .await
        .expect("fetch_and_store");

    assert_eq!(resp.horses_fetched, 1);
    assert_eq!(resp.runs_saved, 1, "同一 netkeiba_race_id は 1 走に集約");
    let calls = interactor.repository.upserted.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1.len(), 1);
}
