//! Offline tests for the netkeiba card/odds ingest interactor (`CardInteractor`).
//!
//! netkeiba is unreachable from CI, so the scraper and repository are faked.
//! These cover the orchestration branches that the parse-layer tests don't:
//! fetch-history dedup, `--force` refetch, "odds always refetched", and the
//! empty-odds (pre-race) skip.

use std::sync::Mutex;

use chrono::NaiveDate;
use paddock_domain::horse_result::{GateNum, HorseName, HorseNum};
use paddock_domain::{JockeyName, RaceCard, RaceId, Surface, TrainerName, Venue};
use paddock_domain::{OrderedPair, OrderedTriple, Pair, Triple};
use paddock_use_case::netkeiba_scraper::{
    FetchedCard, FetchedComboOdds, FetchedEntry, FetchedExoticOdds, FetchedOdds, FetchedPlaceOdds,
    FetchedWinOdds, HorsePastRun, NetkeibaScraper, RunnerRef,
};
use paddock_use_case::repository::{
    FetchRecord, FetchRepository, OddsRepository, RaceCardRepository, RaceOddsRecord,
};
use paddock_use_case::{CardInteractor, Result};

const NK_ID: &str = "202605030211";

// --- fakes ---------------------------------------------------------------

struct FakeScraper {
    /// fetch_win_place_odds が返す単勝オッズ（空ならレース前=未確定を模す）。
    win: Vec<FetchedWinOdds>,
    /// fetch_win_place_odds が返す複勝オッズ。
    place: Vec<FetchedPlaceOdds>,
    /// fetch_exotic_odds が返す組合せ券種オッズ（#102）。
    exotic: FetchedExoticOdds,
    /// true なら fetch_exotic_odds が Err を返す（組合せ取得失敗のベストエフォート検証用）。
    exotic_err: bool,
    /// fetch_card が呼ばれた回数。
    card_fetches: Mutex<usize>,
}

impl FakeScraper {
    fn new(win: Vec<FetchedWinOdds>) -> Self {
        Self {
            win,
            place: Vec::new(),
            exotic: FetchedExoticOdds::default(),
            exotic_err: false,
            card_fetches: Mutex::new(0),
        }
    }

    fn with_place(mut self, place: Vec<FetchedPlaceOdds>) -> Self {
        self.place = place;
        self
    }

    fn with_exotic(mut self, exotic: FetchedExoticOdds) -> Self {
        self.exotic = exotic;
        self
    }

    fn with_exotic_err(mut self) -> Self {
        self.exotic_err = true;
        self
    }
}

fn entry(gate: u32, num: u32, name: &str, jockey: &str, trainer: &str) -> FetchedEntry {
    FetchedEntry {
        gate_num: GateNum::try_from(gate).unwrap(),
        horse_num: HorseNum::try_from(num).unwrap(),
        horse_name: HorseName::try_from(name).unwrap(),
        // 馬番から決定的に 10 桁の netkeiba horse_id を作る（テスト用ダミー）。
        horse_id: Some(paddock_domain::HorseId::try_from(format!("2020{num:06}")).unwrap()),
        jockey: Some(JockeyName::try_from(jockey).unwrap()),
        trainer: Some(TrainerName::try_from(trainer).unwrap()),
        weight_carried: Some(57.0),
    }
}

fn win_odds(num: u32, odds: f64, pop: u32) -> FetchedWinOdds {
    FetchedWinOdds {
        horse_num: HorseNum::try_from(num).unwrap(),
        odds,
        popularity: Some(pop),
    }
}

fn place_odds(num: u32, low: f64, high: f64, pop: u32) -> FetchedPlaceOdds {
    FetchedPlaceOdds {
        horse_num: HorseNum::try_from(num).unwrap(),
        odds_low: low,
        odds_high: high,
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
                entry(1, 1, "ウマエー", "戸崎圭", "藤沢和"),
                entry(2, 2, "ウマビー", "武豊", "友道康"),
            ],
        })
    }
    fn fetch_win_place_odds(&self, _race_id: &str) -> Result<FetchedOdds> {
        Ok(FetchedOdds {
            win: self.win.clone(),
            place: self.place.clone(),
        })
    }
    fn fetch_exotic_odds(&self, _race_id: &str) -> Result<FetchedExoticOdds> {
        if self.exotic_err {
            return Err(paddock_use_case::Error::Internal(
                "exotic odds API down".into(),
            ));
        }
        Ok(self.exotic.clone())
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

impl FetchRepository for RecordingRepo {
    async fn fetch_history_contains(&self, _source_key: &str) -> Result<bool> {
        Ok(self.already)
    }
    async fn record_fetch(&self, record: &FetchRecord) -> Result<()> {
        self.fetch_records.lock().unwrap().push(record.clone());
        Ok(())
    }
    async fn fetch_status(
        &self,
        _source_key: &str,
    ) -> Result<Option<paddock_use_case::repository::FetchStatus>> {
        Ok(self
            .already
            .then_some(paddock_use_case::repository::FetchStatus::Ingested))
    }
    async fn record_download(
        &self,
        _record: &paddock_use_case::repository::FetchDownload,
    ) -> Result<()> {
        Ok(())
    }
    async fn record_failure(
        &self,
        _record: &paddock_use_case::repository::FetchFailure,
    ) -> Result<()> {
        Ok(())
    }
}

impl RaceCardRepository for RecordingRepo {
    async fn save_race_card(&self, card: &RaceCard) -> Result<()> {
        self.saved_cards.lock().unwrap().push(card.clone());
        Ok(())
    }
    async fn find_race_card(&self, _race_id: &RaceId) -> Result<Option<RaceCard>> {
        unimplemented!()
    }
}

impl OddsRepository for RecordingRepo {
    async fn save_race_odds(&self, record: &RaceOddsRecord) -> Result<()> {
        self.saved_odds.lock().unwrap().push(record.clone());
        Ok(())
    }
    async fn find_race_odds(
        &self,
        _race_id: &RaceId,
        _as_of: Option<NaiveDate>,
    ) -> Result<Option<paddock_domain::RaceOdds>> {
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
    // card 取得時は各馬の horse_id を返す（近走取り込み #103 の再利用キー）。
    assert_eq!(resp.horse_ids, vec!["2020000001", "2020000002"]);
    assert_eq!(interactor.repo.saved_cards.lock().unwrap().len(), 1);
    assert_eq!(interactor.repo.fetch_records.lock().unwrap().len(), 1);
    let odds = interactor.repo.saved_odds.lock().unwrap();
    assert_eq!(odds.len(), 1);
    assert_eq!(odds[0].rows.len(), 2);
    assert!(odds[0].rows.iter().all(|r| r.bet_type == "win"));
}

#[tokio::test]
async fn saves_win_and_place_odds_in_one_record() {
    // 単勝 2 行 + 複勝 2 行を 1 レコードにまとめて保存する。複勝は幅 odds（low=odds, high=odds_high）。
    let scraper = FakeScraper::new(vec![win_odds(1, 7.9, 3), win_odds(2, 2.9, 1)])
        .with_place(vec![place_odds(1, 2.6, 4.1, 3), place_odds(2, 1.3, 1.5, 1)]);
    let interactor = CardInteractor::new(RecordingRepo::with_already(false), scraper);

    let resp = interactor.ingest(NK_ID, race_id(), false).await.unwrap();

    assert_eq!(resp.odds_saved, 4, "単勝 2 + 複勝 2");
    let odds = interactor.repo.saved_odds.lock().unwrap();
    assert_eq!(odds.len(), 1, "win/place は 1 レコードにまとめる");
    let rows = &odds[0].rows;
    assert_eq!(rows.iter().filter(|r| r.bet_type == "win").count(), 2);
    let place: Vec<_> = rows.iter().filter(|r| r.bet_type == "place").collect();
    assert_eq!(place.len(), 2);
    // 馬番 1 の複勝: odds=2.6(low), odds_high=4.1(high)。
    let p1 = place.iter().find(|r| r.combination_key == "1").unwrap();
    assert!((p1.odds - 2.6).abs() < 1e-9);
    assert_eq!(p1.odds_high, Some(4.1));
    assert_eq!(p1.popularity, Some(3));
}

#[tokio::test]
async fn saves_exotic_odds_with_combination_keys() {
    // #102: 馬連・馬単・三連複・三連単も単複と同じ 1 レコードに保存する。
    let h = |n: u32| HorseNum::try_from(n).unwrap();
    // 単一クロージャは K を 1 つに固定してしまうため、組合せ型ごとに構築する小関数を使う。
    fn combo<K>(combination: K, odds: f64) -> FetchedComboOdds<K> {
        FetchedComboOdds {
            combination,
            odds,
            popularity: None,
        }
    }
    let exotic = FetchedExoticOdds {
        quinella: vec![combo(Pair::try_from((h(4), h(7))).unwrap(), 21.6)],
        exacta: vec![combo(OrderedPair::try_from((h(7), h(4))).unwrap(), 31.0)],
        trio: vec![combo(Triple::try_from((h(4), h(7), h(13))).unwrap(), 32.9)],
        trifecta: vec![combo(
            OrderedTriple::try_from((h(7), h(4), h(13))).unwrap(),
            154.6,
        )],
    };
    let scraper = FakeScraper::new(vec![win_odds(7, 2.6, 1)]).with_exotic(exotic);
    let interactor = CardInteractor::new(RecordingRepo::with_already(false), scraper);

    let resp = interactor.ingest(NK_ID, race_id(), false).await.unwrap();

    // 単勝 1 + 馬連 1 + 馬単 1 + 三連複 1 + 三連単 1 = 5 行。
    assert_eq!(resp.odds_saved, 5);
    let odds = interactor.repo.saved_odds.lock().unwrap();
    let rows = &odds[0].rows;
    let key_of = |bt: &str| {
        rows.iter()
            .find(|r| r.bet_type == bt)
            .unwrap()
            .combination_key
            .clone()
    };
    assert_eq!(key_of("quinella"), "4-7");
    assert_eq!(key_of("exacta"), "7>4"); // 順序保持
    assert_eq!(key_of("trio"), "4-7-13");
    assert_eq!(key_of("trifecta"), "7>4>13");
}

#[tokio::test]
async fn exotic_fetch_error_still_saves_win_place() {
    // #102: 組合せ券種の取得が失敗しても、確定済みの単複は保存される（ベストエフォート）。
    let scraper = FakeScraper::new(vec![win_odds(1, 7.9, 3), win_odds(2, 2.9, 1)])
        .with_place(vec![place_odds(1, 2.6, 4.1, 3)])
        .with_exotic_err();
    let interactor = CardInteractor::new(RecordingRepo::with_already(false), scraper);

    let resp = interactor.ingest(NK_ID, race_id(), false).await.unwrap();

    // 単勝 2 + 複勝 1 = 3 行。組合せは取得失敗で 0 行だが ingest は成功する。
    assert_eq!(resp.odds_saved, 3);
    let odds = interactor.repo.saved_odds.lock().unwrap();
    let rows = &odds[0].rows;
    assert_eq!(rows.iter().filter(|r| r.bet_type == "win").count(), 2);
    assert_eq!(rows.iter().filter(|r| r.bet_type == "place").count(), 1);
    assert!(
        rows.iter()
            .all(|r| r.bet_type == "win" || r.bet_type == "place"),
        "組合せ券種の行は無い"
    );
}

#[tokio::test]
async fn skips_card_when_already_fetched_but_still_saves_odds() {
    let scraper = FakeScraper::new(vec![win_odds(1, 7.9, 3)]);
    let interactor = CardInteractor::new(RecordingRepo::with_already(true), scraper);

    let resp = interactor.ingest(NK_ID, race_id(), false).await.unwrap();

    assert!(!resp.card_saved, "取得済みなのでカードはスキップ");
    assert_eq!(resp.entries_saved, 0);
    // 取得済みスキップ時は horse_id を採れない → 空（呼び出し側は出馬表から引き直す）。
    assert!(resp.horse_ids.is_empty());
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
