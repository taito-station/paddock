//! backtest interactor のオーケストレーション検証。
//!
//! in-memory モックで、確定レース 1 件に対しトップ選好馬の選択・的中突合・想定回収率が
//! 期待どおり集計されることを確認する（指標計算自体は domain 側で単体テスト済み）。

use chrono::NaiveDate;
use paddock_domain::horse_result::{FinishingPosition, GateNum, HorseName, HorseNum, ResultStatus};
use paddock_domain::{HorseResult, JockeyName, Race, RaceCard, RaceId, Surface, Venue};
use paddock_use_case::repository::{
    CourseStatsRow, FetchRecord, GroupStat, HorseStatsRow, JockeyStatsRow, Repository,
};
use paddock_use_case::{Interactor, Result};

fn make_group(label: &str, starts: u32, wins: u32) -> GroupStat {
    GroupStat {
        label: label.to_string(),
        starts,
        wins,
        places: wins,
        shows: wins,
    }
}

fn horse_stats_with_surface_win(win_rate: f64) -> HorseStatsRow {
    let starts = 10;
    let wins = (win_rate * starts as f64).round() as u32;
    HorseStatsRow {
        horse_name: String::new(),
        by_surface: vec![make_group("芝", starts, wins), make_group("ダート", 5, 0)],
        by_distance_band: vec![
            make_group("〜1400m", 0, 0),
            make_group("1500〜1800m", 0, 0),
            make_group("1900〜2200m", starts, wins),
            make_group("2300m〜", 0, 0),
        ],
        by_gate_group: vec![],
        by_track_condition: vec![],
        by_popularity_band: vec![],
        overall: make_group("全体", starts, wins),
    }
}

fn course_stats() -> CourseStatsRow {
    CourseStatsRow {
        venue: "中山".to_string(),
        distance: 2000,
        surface: "turf".to_string(),
        by_gate_group: vec![
            make_group("Inner (1-3)", 20, 4),
            make_group("Middle (4-6)", 20, 2),
            make_group("Outer (7-8)", 20, 1),
        ],
    }
}

fn result(horse_num: u32, gate: u32, horse: &str, finish: u32, odds: Option<f64>) -> HorseResult {
    HorseResult {
        finishing_position: Some(FinishingPosition::try_from(finish).unwrap()),
        status: ResultStatus::Finished,
        gate_num: GateNum::try_from(gate).unwrap(),
        horse_num: HorseNum::try_from(horse_num).unwrap(),
        horse_name: HorseName::try_from(horse).unwrap(),
        horse_id: None,
        jockey: None,
        trainer: None,
        time_seconds: None,
        margin: None,
        odds,
        horse_weight: None,
        weight_change: None,
        weight_carried: None,
        popularity: None,
    }
}

/// 出走取消・競走除外（着順なし・status 指定）の馬。
fn non_starter(horse_num: u32, gate: u32, horse: &str, status: ResultStatus) -> HorseResult {
    HorseResult {
        finishing_position: None,
        status,
        gate_num: GateNum::try_from(gate).unwrap(),
        horse_num: HorseNum::try_from(horse_num).unwrap(),
        horse_name: HorseName::try_from(horse).unwrap(),
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
    }
}

fn finished_race() -> Race {
    Race {
        race_id: RaceId::try_from("2026-1-nakayama-1-1R").unwrap(),
        date: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
        venue: Venue::Nakayama,
        round: 1,
        day: 1,
        race_num: 1,
        surface: Surface::Turf,
        distance: 2000,
        track_condition: None,
        weather: None,
        // ウマA(枠1, Inner, 高スタッツ)が 1 着・オッズ 4.0、ウマB(枠5, Middle, 低スタッツ)が 2 着。
        results: vec![
            result(1, 1, "ウマA", 1, Some(4.0)),
            result(2, 5, "ウマB", 2, None),
        ],
    }
}

struct MockRepo {
    races: Vec<Race>,
}

impl Repository for MockRepo {
    async fn find_finished_races_between(
        &self,
        _from: NaiveDate,
        _to: NaiveDate,
    ) -> Result<Vec<Race>> {
        Ok(self.races.clone())
    }

    async fn find_recent_runs(
        &self,
        _name: &HorseName,
        _before: NaiveDate,
        _limit: u32,
    ) -> Result<Vec<(NaiveDate, HorseResult)>> {
        Ok(Vec::new())
    }

    async fn horse_stats(
        &self,
        name: &HorseName,
        _as_of: Option<NaiveDate>,
    ) -> Result<HorseStatsRow> {
        // ウマA を高スタッツでトップ選好馬に。ウマS は最高スタッツだが（テストで）出走取消なので
        // 除外され、確率推定の母集合に入らないことを検証する。
        let win_rate = match name.value() {
            "ウマA" => 0.3,
            "ウマS" => 0.9,
            _ => 0.05,
        };
        Ok(horse_stats_with_surface_win(win_rate))
    }

    async fn course_stats(
        &self,
        _: Venue,
        _: u32,
        _: Surface,
        _as_of: Option<NaiveDate>,
    ) -> Result<CourseStatsRow> {
        Ok(course_stats())
    }

    async fn jockey_stats(
        &self,
        _: &JockeyName,
        _as_of: Option<NaiveDate>,
    ) -> Result<JockeyStatsRow> {
        unimplemented!()
    }

    // --- 以下は backtest では未使用 ---
    async fn save_race(&self, _: &Race) -> Result<()> {
        unimplemented!()
    }
    async fn upsert_history_race(&self, _: &Race) -> Result<()> {
        unimplemented!()
    }
    async fn count_races(&self) -> Result<u64> {
        unimplemented!()
    }
    async fn race_exists(&self, _: &RaceId) -> Result<bool> {
        unimplemented!()
    }
    async fn fetch_history_contains(&self, _: &str) -> Result<bool> {
        unimplemented!()
    }
    async fn record_fetch(&self, _: &FetchRecord) -> Result<()> {
        unimplemented!()
    }
    async fn save_race_card(&self, _: &RaceCard) -> Result<()> {
        unimplemented!()
    }
    async fn find_race_card(&self, _: &RaceId) -> Result<Option<RaceCard>> {
        unimplemented!()
    }
    async fn find_races_by_date(&self, _: NaiveDate) -> Result<Vec<Race>> {
        unimplemented!()
    }
    async fn find_predict_session(
        &self,
        _: NaiveDate,
    ) -> Result<Option<paddock_use_case::repository::PredictSessionRecord>> {
        unimplemented!()
    }
    async fn find_predict_bets(
        &self,
        _: NaiveDate,
    ) -> Result<Vec<paddock_use_case::repository::PredictBetRecord>> {
        unimplemented!()
    }
    async fn save_predict_session(
        &self,
        _: &paddock_use_case::repository::PredictSessionRecord,
    ) -> Result<()> {
        unimplemented!()
    }
    async fn save_race_outcome(
        &self,
        _: &paddock_use_case::repository::PredictSessionRecord,
        _: &RaceId,
        _: &[paddock_use_case::repository::PredictBetRecord],
    ) -> Result<()> {
        unimplemented!()
    }
}

struct NullParser;
impl paddock_use_case::PdfParser for NullParser {
    fn parse(&self, _: &[u8]) -> Result<Vec<Race>> {
        unimplemented!()
    }
}

struct NullFetcher;
impl paddock_use_case::PdfFetcher for NullFetcher {
    fn fetch(&self, _: &str) -> Result<Vec<u8>> {
        unimplemented!()
    }
    fn fetch_if_exists(&self, _: &str) -> Result<Option<Vec<u8>>> {
        unimplemented!()
    }
}

fn interactor(races: Vec<Race>) -> Interactor<MockRepo, NullParser, NullFetcher> {
    Interactor::new(MockRepo { races }, NullParser, NullFetcher)
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

#[tokio::test]
async fn backtest_aggregates_top_pick_and_payout() {
    let app = interactor(vec![finished_race()]);
    let report = app.backtest(d(2026, 1, 1), d(2026, 1, 31)).await.unwrap();

    assert_eq!(report.races_evaluated, 1);
    // トップ選好馬は高スタッツの ウマA(1 着) → 単勝・連対・複勝すべて的中
    assert!((report.win_hit_rate - 1.0).abs() < 1e-9);
    assert!((report.place_hit_rate - 1.0).abs() < 1e-9);
    assert!((report.show_hit_rate - 1.0).abs() < 1e-9);
    // ウマA のオッズ 4.0 で 1 着 → 回収率 400/100 = 4.0、母数 1
    assert_eq!(report.payout_races, 1);
    assert!((report.payout_rate.unwrap() - 4.0).abs() < 1e-9);
    // キャリブレーション指標は有限・非負
    assert!(report.brier.is_finite() && report.brier >= 0.0);
    assert!(report.log_loss.is_finite() && report.log_loss >= 0.0);
}

#[tokio::test]
async fn backtest_empty_when_no_races() {
    let app = interactor(Vec::new());
    let report = app.backtest(d(2026, 1, 1), d(2026, 1, 31)).await.unwrap();
    assert_eq!(report.races_evaluated, 0);
    assert!(report.payout_rate.is_none());
}

#[tokio::test]
async fn backtest_excludes_scratched_and_cancelled_horses() {
    // ウマS は最高スタッツ(0.9)だが出走取消、ウマC は競走除外。両者が母集合に混入すると
    // ウマS がトップ選好馬になり着順なし → 単勝的中率 0 になってしまう。除外されていれば
    // 発走馬の中で最高スタッツの ウマA(1 着)がトップ選好馬となり単勝的中率 1.0 になる。
    let race = Race {
        race_id: RaceId::try_from("2026-1-nakayama-1-2R").unwrap(),
        date: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
        venue: Venue::Nakayama,
        round: 1,
        day: 1,
        race_num: 2,
        surface: Surface::Turf,
        distance: 2000,
        track_condition: None,
        weather: None,
        results: vec![
            result(1, 1, "ウマA", 1, Some(4.0)),
            result(2, 5, "ウマB", 2, None),
            non_starter(3, 2, "ウマS", ResultStatus::Cancelled),
            non_starter(4, 3, "ウマC", ResultStatus::Scratched),
        ],
    };
    let app = interactor(vec![race]);
    let report = app.backtest(d(2026, 1, 1), d(2026, 1, 31)).await.unwrap();

    assert_eq!(report.races_evaluated, 1);
    // 非出走馬が除外され、発走馬の最高スタッツ ウマA が 1 着 → 単勝的中
    assert!(
        (report.win_hit_rate - 1.0).abs() < 1e-9,
        "出走取消・競走除外が母集合に混入している (win_hit_rate={})",
        report.win_hit_rate
    );
}
