//! Offline tests for the netkeiba card/odds ingest interactor (`CardInteractor`).
//!
//! netkeiba is unreachable from CI, so the scraper and repository are faked.
//! These cover the orchestration branches that the parse-layer tests don't:
//! fetch-history dedup, `--force` refetch, "odds always refetched", and the
//! empty-odds (pre-race) skip.

use std::sync::Mutex;

use chrono::NaiveDate;
use paddock_domain::horse_result::{GateNum, HorseName, HorseNum};
use paddock_domain::{JockeyName, Race, RaceCard, RaceId, Surface, Venue};
use paddock_use_case::netkeiba_scraper::{
    FetchedCard, FetchedEntry, FetchedWinOdds, HorsePastRun, NetkeibaScraper, RunnerRef,
};
use paddock_use_case::repository::{
    CourseStatsRow, FetchRecord, HorseStatsRow, JockeyStatsRow, PredictBetRecord,
    PredictSessionRecord, RaceOddsRecord, Repository,
};
use paddock_use_case::{CardInteractor, Result};

const NK_ID: &str = "202605030211";

// --- fakes ---------------------------------------------------------------

struct FakeScraper {
    /// fetch_win_odds が返す単勝オッズ（空ならレース前=未確定を模す）。
    win: Vec<FetchedWinOdds>,
    /// fetch_card が呼ばれた回数。
    card_fetches: Mutex<usize>,
}

impl FakeScraper {
    fn new(win: Vec<FetchedWinOdds>) -> Self {
        Self {
            win,
            card_fetches: Mutex::new(0),
        }
    }
}

fn entry(gate: u32, num: u32, name: &str, jockey: &str) -> FetchedEntry {
    FetchedEntry {
        gate_num: GateNum::try_from(gate).unwrap(),
        horse_num: HorseNum::try_from(num).unwrap(),
        horse_name: HorseName::try_from(name).unwrap(),
        jockey: Some(JockeyName::try_from(jockey).unwrap()),
    }
}

fn win_odds(num: u32, odds: f64, pop: u32) -> FetchedWinOdds {
    FetchedWinOdds {
        horse_num: HorseNum::try_from(num).unwrap(),
        odds,
        popularity: Some(pop),
    }
}

impl NetkeibaScraper for FakeScraper {
    fn fetch_shutuba(&self, _race_id: &str) -> Result<Vec<RunnerRef>> {
        unimplemented!()
    }
    fn fetch_horse_history(
        &self,
        _horse_id: &paddock_domain::HorseId,
    ) -> Result<Vec<HorsePastRun>> {
        unimplemented!()
    }
    fn fetch_card(&self, _race_id: &str) -> Result<FetchedCard> {
        *self.card_fetches.lock().unwrap() += 1;
        Ok(FetchedCard {
            date: NaiveDate::from_ymd_opt(2026, 6, 7).unwrap(),
            venue: Venue::Tokyo,
            round: 3,
            day: 2,
            race_num: 11,
            surface: Surface::Turf,
            distance: 1600,
            entries: vec![
                entry(1, 1, "ウマエー", "戸崎圭"),
                entry(2, 2, "ウマビー", "武豊"),
            ],
        })
    }
    fn fetch_win_odds(&self, _race_id: &str) -> Result<Vec<FetchedWinOdds>> {
        Ok(self.win.clone())
    }
}

#[derive(Default)]
struct RecordingRepo {
    /// fetch_history_contains の戻り値（出馬表が取得済みか）。
    already: bool,
    saved_cards: Mutex<Vec<RaceCard>>,
    saved_odds: Mutex<Vec<RaceOddsRecord>>,
    fetch_records: Mutex<Vec<FetchRecord>>,
}

impl RecordingRepo {
    fn with_already(already: bool) -> Self {
        Self {
            already,
            ..Default::default()
        }
    }
}

impl Repository for RecordingRepo {
    async fn fetch_history_contains(&self, _source_key: &str) -> Result<bool> {
        Ok(self.already)
    }
    async fn record_fetch(&self, record: &FetchRecord) -> Result<()> {
        self.fetch_records.lock().unwrap().push(record.clone());
        Ok(())
    }
    async fn save_race_card(&self, card: &RaceCard) -> Result<()> {
        self.saved_cards.lock().unwrap().push(card.clone());
        Ok(())
    }
    async fn save_race_odds(&self, record: &RaceOddsRecord) -> Result<()> {
        self.saved_odds.lock().unwrap().push(record.clone());
        Ok(())
    }
    // --- 以降は本テストで未使用 ---
    async fn save_race(&self, _race: &Race) -> Result<()> {
        unimplemented!()
    }
    async fn find_recent_runs(
        &self,
        _name: &HorseName,
        _before: NaiveDate,
        _limit: u32,
    ) -> Result<Vec<(NaiveDate, paddock_domain::horse_result::HorseResult)>> {
        unimplemented!()
    }
    async fn upsert_horse_history(
        &self,
        _horse_id: &paddock_domain::HorseId,
        _runs: &[paddock_use_case::HorsePastRun],
    ) -> Result<()> {
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

fn race_id() -> RaceId {
    RaceId::try_from("2026-3-tokyo-2-11R".to_string()).unwrap()
}

// --- tests ---------------------------------------------------------------

#[tokio::test]
async fn fresh_run_saves_card_and_odds() {
    let scraper = FakeScraper::new(vec![win_odds(1, 7.9, 3), win_odds(2, 2.9, 1)]);
    let interactor = CardInteractor::new(RecordingRepo::with_already(false), scraper);

    let resp = interactor.ingest(NK_ID, race_id(), false).await.unwrap();

    assert!(resp.card_saved);
    assert_eq!(resp.entries_saved, 2);
    assert_eq!(resp.odds_saved, 2);
    assert_eq!(interactor.repo.saved_cards.lock().unwrap().len(), 1);
    assert_eq!(interactor.repo.fetch_records.lock().unwrap().len(), 1);
    let odds = interactor.repo.saved_odds.lock().unwrap();
    assert_eq!(odds.len(), 1);
    assert_eq!(odds[0].rows.len(), 2);
    assert!(odds[0].rows.iter().all(|r| r.bet_type == "win"));
}

#[tokio::test]
async fn skips_card_when_already_fetched_but_still_saves_odds() {
    let scraper = FakeScraper::new(vec![win_odds(1, 7.9, 3)]);
    let interactor = CardInteractor::new(RecordingRepo::with_already(true), scraper);

    let resp = interactor.ingest(NK_ID, race_id(), false).await.unwrap();

    assert!(!resp.card_saved, "取得済みなのでカードはスキップ");
    assert_eq!(resp.entries_saved, 0);
    assert_eq!(
        *interactor.scraper.card_fetches.lock().unwrap(),
        0,
        "fetch_card は呼ばれない"
    );
    assert!(interactor.repo.saved_cards.lock().unwrap().is_empty());
    assert!(interactor.repo.fetch_records.lock().unwrap().is_empty());
    // オッズは変動するため取得済みでも常に保存する。
    assert_eq!(resp.odds_saved, 1);
    assert_eq!(interactor.repo.saved_odds.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn force_refetches_card_even_if_already_present() {
    let scraper = FakeScraper::new(vec![win_odds(1, 7.9, 3)]);
    let interactor = CardInteractor::new(RecordingRepo::with_already(true), scraper);

    let resp = interactor.ingest(NK_ID, race_id(), true).await.unwrap();

    assert!(resp.card_saved, "--force で取得済みでも再取得");
    assert_eq!(*interactor.scraper.card_fetches.lock().unwrap(), 1);
    assert_eq!(interactor.repo.saved_cards.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn empty_odds_skips_odds_save() {
    let scraper = FakeScraper::new(vec![]); // レース前で未確定
    let interactor = CardInteractor::new(RecordingRepo::with_already(false), scraper);

    let resp = interactor.ingest(NK_ID, race_id(), false).await.unwrap();

    assert!(resp.card_saved);
    assert_eq!(resp.odds_saved, 0);
    assert!(
        interactor.repo.saved_odds.lock().unwrap().is_empty(),
        "空オッズでは save_race_odds を呼ばない"
    );
}
