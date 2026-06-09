//! backtest interactor のオーケストレーション検証。
//!
//! in-memory モックで、確定レース 1 件に対しトップ選好馬の選択・的中突合・想定回収率が
//! 期待どおり集計されることを確認する（指標計算自体は domain 側で単体テスト済み）。

use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::horse_result::{FinishingPosition, GateNum, HorseName, HorseNum, ResultStatus};
use paddock_domain::{
    HorseResult, JockeyName, OddsValue, Race, RaceCard, RaceId, RaceOdds, Surface, Venue,
};
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
    /// race_id 値 → 保存済みオッズ。backtest の「当時オッズ参照」を模す。
    race_odds: HashMap<String, RaceOdds>,
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
    async fn upsert_horse_history(
        &self,
        _: &paddock_domain::HorseId,
        _: &[paddock_use_case::HorsePastRun],
    ) -> Result<()> {
        unimplemented!()
    }
    async fn backfill_results_horse_ids(&self) -> Result<u64> {
        unimplemented!()
    }
    async fn find_matching_horse_names(&self, _query: &str, _limit: u32) -> Result<Vec<String>> {
        unimplemented!()
    }
    async fn find_matching_jockey_names(&self, _query: &str, _limit: u32) -> Result<Vec<String>> {
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
    async fn save_race_odds(&self, _: &paddock_use_case::repository::RaceOddsRecord) -> Result<()> {
        unimplemented!()
    }
    async fn find_race_card(&self, _: &RaceId) -> Result<Option<RaceCard>> {
        unimplemented!()
    }
    async fn find_race_odds(
        &self,
        race_id: &RaceId,
        _as_of: Option<NaiveDate>,
    ) -> Result<Option<RaceOdds>> {
        Ok(self.race_odds.get(race_id.value()).cloned())
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
    interactor_with_odds(races, HashMap::new())
}

fn interactor_with_odds(
    races: Vec<Race>,
    race_odds: HashMap<String, RaceOdds>,
) -> Interactor<MockRepo, NullParser, NullFetcher> {
    Interactor::new(MockRepo { races, race_odds }, NullParser, NullFetcher)
}

/// 1 頭分の単勝オッズだけを持つ RaceOdds を作る（当時オッズ参照テスト用）。
fn win_only_odds(race_id: &str, horse_num: u32, odds: f64) -> RaceOdds {
    let mut o = RaceOdds::empty(RaceId::try_from(race_id).unwrap());
    o.win.insert(
        HorseNum::try_from(horse_num).unwrap(),
        OddsValue::try_from(odds).unwrap(),
    );
    o
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

#[tokio::test]
async fn backtest_aggregates_top_pick_and_payout() {
    let app = interactor(vec![finished_race()]);
    let report = app.backtest(d(2026, 1, 1), d(2026, 1, 31), None).await.unwrap();

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
async fn backtest_prefers_market_odds_over_pdf() {
    // race_odds に当時オッズ（単勝 7.0）があれば PDF 成績の単勝(4.0)よりそちらを採用する(#51)。
    let race = finished_race();
    let mut odds = HashMap::new();
    odds.insert(
        race.race_id.value().to_string(),
        win_only_odds(race.race_id.value(), 1, 7.0),
    );
    let app = interactor_with_odds(vec![race], odds);
    let report = app.backtest(d(2026, 1, 1), d(2026, 1, 31), None).await.unwrap();

    assert_eq!(report.payout_races, 1);
    // トップ選好馬 ウマA(1 着) を当時オッズ 7.0 で計上 → 回収率 7.0（PDF の 4.0 ではない）
    assert!(
        (report.payout_rate.unwrap() - 7.0).abs() < 1e-9,
        "当時オッズが使われていない (payout_rate={:?})",
        report.payout_rate
    );
}

/// モデルは高スタッツの ウマA を本命にするが、ウマA は 2 着で、低スタッツの ウマB(市場の
/// 圧倒的人気)が 1 着のレース。ブレンドで本命が市場人気の ウマB に動くことを検証する。
fn blend_race() -> Race {
    Race {
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
            result(1, 1, "ウマA", 2, Some(9.0)), // 高スタッツ・2 着・人気薄
            result(2, 5, "ウマB", 1, Some(1.2)), // 低スタッツ・1 着・圧倒的人気
        ],
    }
}

#[tokio::test]
async fn backtest_blend_flips_top_pick_to_market_favorite() {
    // モデルのみ: 本命 ウマA は 2 着 → 単勝的中 0。
    let model_only = interactor(vec![blend_race()])
        .backtest(d(2026, 1, 1), d(2026, 1, 31), None)
        .await
        .unwrap();
    assert!(
        model_only.win_hit_rate.abs() < 1e-9,
        "model-only win_hit={}",
        model_only.win_hit_rate
    );

    // 当時 race_odds(ウマB=1.2 で圧倒的人気)とブレンド(α=0.2)→ 本命が ウマB に動き 1 着的中。
    let race = blend_race();
    let mut odds = HashMap::new();
    let mut o = RaceOdds::empty(RaceId::try_from(race.race_id.value()).unwrap());
    o.win
        .insert(HorseNum::try_from(1u32).unwrap(), OddsValue::try_from(9.0).unwrap());
    o.win
        .insert(HorseNum::try_from(2u32).unwrap(), OddsValue::try_from(1.2).unwrap());
    odds.insert(race.race_id.value().to_string(), o);

    let blended = interactor_with_odds(vec![race], odds)
        .backtest(d(2026, 1, 1), d(2026, 1, 31), Some(0.2))
        .await
        .unwrap();
    assert!(
        (blended.win_hit_rate - 1.0).abs() < 1e-9,
        "blended win_hit={}",
        blended.win_hit_rate
    );
}

#[tokio::test]
async fn backtest_blend_uses_partial_race_odds_as_is() {
    // race_odds.win に勝ち馬 ウマB のみ（部分カバレッジ）。win が非空なので results.odds へは
    // フォールバックせず race_odds をそのまま使い、domain の部分カバレッジ処理（カバー馬に市場重みが
    // 乗る既知挙動）で本命が ウマB に動き 1 着的中する。full-coverage 前提の既知挙動を固定するテスト。
    let race = blend_race();
    let mut odds = HashMap::new();
    let mut o = RaceOdds::empty(RaceId::try_from(race.race_id.value()).unwrap());
    o.win
        .insert(HorseNum::try_from(2u32).unwrap(), OddsValue::try_from(1.2).unwrap());
    odds.insert(race.race_id.value().to_string(), o);

    let report = interactor_with_odds(vec![race], odds)
        .backtest(d(2026, 1, 1), d(2026, 1, 31), Some(0.2))
        .await
        .unwrap();
    assert!(
        (report.win_hit_rate - 1.0).abs() < 1e-9,
        "partial race_odds win_hit={}",
        report.win_hit_rate
    );
}

#[tokio::test]
async fn backtest_blend_falls_back_to_results_odds_when_no_snapshot() {
    // race_odds スナップショット無し → PDF 確定成績の単勝(ウマB=1.2)で代替し、市場のみ(α=0)で
    // 本命が ウマB に動き 1 着的中する。
    let blended = interactor(vec![blend_race()])
        .backtest(d(2026, 1, 1), d(2026, 1, 31), Some(0.0))
        .await
        .unwrap();
    assert!(
        (blended.win_hit_rate - 1.0).abs() < 1e-9,
        "fallback blended win_hit={}",
        blended.win_hit_rate
    );
}

#[tokio::test]
async fn backtest_empty_when_no_races() {
    let app = interactor(Vec::new());
    let report = app.backtest(d(2026, 1, 1), d(2026, 1, 31), None).await.unwrap();
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
    let report = app.backtest(d(2026, 1, 1), d(2026, 1, 31), None).await.unwrap();

    assert_eq!(report.races_evaluated, 1);
    // 非出走馬が除外され、発走馬の最高スタッツ ウマA が 1 着 → 単勝的中
    assert!(
        (report.win_hit_rate - 1.0).abs() < 1e-9,
        "出走取消・競走除外が母集合に混入している (win_hit_rate={})",
        report.win_hit_rate
    );
}
