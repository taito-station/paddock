//! backtest interactor のオーケストレーション検証。
//!
//! in-memory モックで、確定レース 1 件に対しトップ選好馬の選択・的中突合・想定回収率が
//! 期待どおり集計されることを確認する（指標計算自体は domain 側で単体テスト済み）。

use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::horse_result::{FinishingPosition, GateNum, HorseName, HorseNum, ResultStatus};
use paddock_domain::{
    DatedCounts, EstimationConfig, HorseResult, JockeyName, OddsValue, Race, RaceCard, RaceId,
    RaceOdds, RecencyConfig, Surface, TrackCondition, TrainerName, Venue,
};
use paddock_use_case::repository::{
    CourseStatsRow, FetchRecord, GroupStat, HorseRecencyStats, HorseStatsRow, JockeyStatsRow,
    RecencySeries, Repository, TrainerStatsRow,
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
        let mut row = horse_stats_with_surface_win(win_rate);
        // ウマB のみ不良馬場の好成績を持たせる（#73 の配線テスト用）。レースの
        // track_condition が None の既存テストでは馬場項が使われないため影響しない。
        if name.value() == "ウマB" {
            row.by_track_condition = vec![make_group("不良", 10, 10)];
        }
        Ok(row)
    }

    async fn horse_recency(
        &self,
        name: &HorseName,
        _as_of: Option<NaiveDate>,
    ) -> Result<HorseRecencyStats> {
        // ウマB のみ直近の芝・距離帯で好成績（recency 配線テスト用, #75 Phase B）。recency 有効時
        // のみ参照され、ウマA は空＝当該 factor None（集計から recency へ差し替わることを検証する）。
        if name.value() == "ウマB" {
            let run = DatedCounts {
                date: d(2026, 1, 5),
                starts: 3,
                wins: 3,
                places: 3,
                shows: 3,
            };
            Ok(HorseRecencyStats {
                by_surface: vec![RecencySeries {
                    label: "芝".to_string(),
                    runs: vec![run],
                }],
                by_distance_band: vec![RecencySeries {
                    label: "1900〜2200m".to_string(),
                    runs: vec![run],
                }],
                by_track_condition: vec![],
            })
        } else {
            Ok(HorseRecencyStats::default())
        }
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

    async fn trainer_stats(
        &self,
        name: &TrainerName,
        _as_of: Option<NaiveDate>,
    ) -> Result<TrainerStatsRow> {
        // 「名伯楽」だけ芝で全勝（#74 の配線テスト用）。それ以外は実績なし（by_surface 空）。
        let by_surface = if name.value() == "名伯楽" {
            vec![make_group("芝", 10, 10)]
        } else {
            vec![]
        };
        Ok(TrainerStatsRow {
            trainer_name: name.value().to_string(),
            overall: make_group("全体", 0, 0),
            by_surface,
            by_gate_group: vec![],
        })
    }

    // --- 以下は backtest では未使用 ---
    async fn save_race(&self, _: &Race) -> Result<()> {
        unimplemented!()
    }
    async fn upsert_horse_history(
        &self,
        _: &paddock_domain::HorseId,
        _: &[paddock_use_case::HorsePastRun],
    ) -> Result<usize> {
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
    async fn find_matching_trainer_names(&self, _query: &str, _limit: u32) -> Result<Vec<String>> {
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
    let report = app
        .backtest(d(2026, 1, 1), d(2026, 1, 31), None, EstimationConfig::default())
        .await
        .unwrap();

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
    let report = app
        .backtest(d(2026, 1, 1), d(2026, 1, 31), None, EstimationConfig::default())
        .await
        .unwrap();

    assert_eq!(report.payout_races, 1);
    // トップ選好馬 ウマA(1 着) を当時オッズ 7.0 で計上 → 回収率 7.0（PDF の 4.0 ではない）
    assert!(
        (report.payout_rate.unwrap() - 7.0).abs() < 1e-9,
        "当時オッズが使われていない (payout_rate={:?})",
        report.payout_rate
    );
}

#[tokio::test]
async fn backtest_populates_by_exotic_from_curated_bets() {
    // 当時オッズ（単勝）があるレースでは select_bets の curated 推奨を確定着順と突合し、
    // by_exotic（券種別 校正・回収率）が埋まる結合経路を検証する（#121, 単体テストは domain 側）。
    let race = finished_race();
    let mut odds = HashMap::new();
    // 本命 ウマA(1 着) に EV>1 かつ Kelly>min_kelly になる単勝オッズを与える。
    odds.insert(
        race.race_id.value().to_string(),
        win_only_odds(race.race_id.value(), 1, 4.0),
    );
    let app = interactor_with_odds(vec![race], odds);
    let report = app
        .backtest(d(2026, 1, 1), d(2026, 1, 31), None, EstimationConfig::default())
        .await
        .unwrap();

    // 単勝の買い目が採用され、本命 ウマA は 1 着なので的中して by_exotic に現れる。
    let win = report
        .by_exotic
        .iter()
        .find(|s| s.label == "win")
        .expect("by_exotic に win セグメントが埋まること");
    assert_eq!(win.bets, 1, "ウマA の単勝 1 点のみ採用");
    assert!((win.hit_rate - 1.0).abs() < 1e-9, "1 着＝的中");
    // 賭け金一定なので回収率 = 的中オッズ = 4.0。
    assert!(
        (win.payout_rate - 4.0).abs() < 1e-9,
        "回収率は的中オッズ (payout_rate={})",
        win.payout_rate
    );
    assert!(win.mean_predicted > 0.0 && win.mean_predicted <= 1.0);
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

/// モデルは高スタッツの ウマA を本命にするが、不良馬場のレースでは道悪巧者の ウマB
/// （`by_track_condition` の不良 10 戦 10 勝）が本命に入れ替わるレース（#73 の配線検証用）。
fn soft_track_race(track_condition: Option<TrackCondition>) -> Race {
    Race {
        race_id: RaceId::try_from("2026-1-nakayama-1-3R").unwrap(),
        date: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
        venue: Venue::Nakayama,
        round: 1,
        day: 1,
        race_num: 3,
        surface: Surface::Turf,
        distance: 2000,
        track_condition,
        weather: None,
        results: vec![
            result(1, 1, "ウマA", 2, Some(9.0)), // 高スタッツ・2 着
            result(2, 5, "ウマB", 1, Some(5.0)), // 低スタッツだが道悪巧者・1 着
        ],
    }
}

#[tokio::test]
async fn backtest_wires_race_track_condition_into_factors() {
    // 馬場状態なし: 本命は高スタッツの ウマA(2 着) → 単勝的中 0。
    let without_tc = interactor(vec![soft_track_race(None)])
        .backtest(d(2026, 1, 1), d(2026, 1, 31), None, EstimationConfig::default())
        .await
        .unwrap();
    assert!(
        without_tc.win_hit_rate.abs() < 1e-9,
        "track_condition なしで本命が動いている (win_hit={})",
        without_tc.win_hit_rate
    );

    // 不良馬場: race.track_condition が build_factors へ配線され、道悪巧者の
    // ウマB(1 着)へ本命が入れ替わる → 単勝的中 1。
    let with_tc = interactor(vec![soft_track_race(Some(TrackCondition::Soft))])
        .backtest(d(2026, 1, 1), d(2026, 1, 31), None, EstimationConfig::default())
        .await
        .unwrap();
    assert!(
        (with_tc.win_hit_rate - 1.0).abs() < 1e-9,
        "race.track_condition が factor に配線されていない (win_hit={})",
        with_tc.win_hit_rate
    );
}

#[tokio::test]
async fn backtest_wires_recency_into_horse_factors() {
    // recency なし: 本命は高スタッツの ウマA(2 着) → 単勝的中 0（集計レート経路）。
    let off = interactor(vec![soft_track_race(None)])
        .backtest(d(2026, 1, 1), d(2026, 1, 31), None, EstimationConfig::default())
        .await
        .unwrap();
    assert!(
        off.win_hit_rate.abs() < 1e-9,
        "recency off で本命が動いている (win_hit={})",
        off.win_hit_rate
    );

    // recency あり: ウマB の直近 芝・距離帯の好成績が horse_recency 経由で
    // build_factors に配線され、本命が ウマB(1 着)へ入れ替わる → 単勝的中 1。
    // ラベル不一致や系列取得漏れがあれば recency factor が None になり off と同じ 0 のままになる。
    let cfg = EstimationConfig {
        shrinkage: None,
        recency: Some(RecencyConfig {
            half_life_days: 30.0,
        }),
    };
    let on = interactor(vec![soft_track_race(None)])
        .backtest(d(2026, 1, 1), d(2026, 1, 31), None, cfg)
        .await
        .unwrap();
    assert!(
        (on.win_hit_rate - 1.0).abs() < 1e-9,
        "recency が horse factor に配線されていない (win_hit={})",
        on.win_hit_rate
    );
}

/// モデルは高スタッツの ウマA を本命にするが、ウマB に名伯楽（mock の trainer_stats が芝 10 戦
/// 10 勝を返す）を付けると本命が ウマB に入れ替わるレース（#74 の配線検証用）。trainer は
/// results.trainer（当該レース確定値）から引く。
fn trainer_race(b_trainer: Option<&str>) -> Race {
    let mut b = result(2, 5, "ウマB", 1, Some(5.0)); // 低スタッツだが名伯楽・1 着
    if let Some(t) = b_trainer {
        b.trainer = Some(TrainerName::try_from(t).unwrap());
    }
    Race {
        race_id: RaceId::try_from("2026-1-nakayama-1-4R").unwrap(),
        date: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
        venue: Venue::Nakayama,
        round: 1,
        day: 1,
        race_num: 4,
        surface: Surface::Turf,
        distance: 2000,
        track_condition: None,
        weather: None,
        results: vec![
            result(1, 1, "ウマA", 2, Some(9.0)), // 高スタッツ・2 着
            b,
        ],
    }
}

#[tokio::test]
async fn backtest_wires_result_trainer_into_factors() {
    // 調教師なし: 本命は高スタッツの ウマA(2 着) → 単勝的中 0。
    let without = interactor(vec![trainer_race(None)])
        .backtest(d(2026, 1, 1), d(2026, 1, 31), None, EstimationConfig::default())
        .await
        .unwrap();
    assert!(
        without.win_hit_rate.abs() < 1e-9,
        "trainer なしで本命が動いている (win_hit={})",
        without.win_hit_rate
    );

    // 名伯楽: results.trainer が build_factors へ配線され、ウマB(1 着)へ本命が入れ替わる
    // → 単勝的中 1。
    let with_tr = interactor(vec![trainer_race(Some("名伯楽"))])
        .backtest(d(2026, 1, 1), d(2026, 1, 31), None, EstimationConfig::default())
        .await
        .unwrap();
    assert!(
        (with_tr.win_hit_rate - 1.0).abs() < 1e-9,
        "results.trainer が factor に配線されていない (win_hit={})",
        with_tr.win_hit_rate
    );
}

#[tokio::test]
async fn backtest_blend_flips_top_pick_to_market_favorite() {
    // モデルのみ: 本命 ウマA は 2 着 → 単勝的中 0。
    let model_only = interactor(vec![blend_race()])
        .backtest(d(2026, 1, 1), d(2026, 1, 31), None, EstimationConfig::default())
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
    o.win.insert(
        HorseNum::try_from(1u32).unwrap(),
        OddsValue::try_from(9.0).unwrap(),
    );
    o.win.insert(
        HorseNum::try_from(2u32).unwrap(),
        OddsValue::try_from(1.2).unwrap(),
    );
    odds.insert(race.race_id.value().to_string(), o);

    let blended = interactor_with_odds(vec![race], odds)
        .backtest(d(2026, 1, 1), d(2026, 1, 31), Some(0.2), EstimationConfig::default())
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
    o.win.insert(
        HorseNum::try_from(2u32).unwrap(),
        OddsValue::try_from(1.2).unwrap(),
    );
    odds.insert(race.race_id.value().to_string(), o);

    let report = interactor_with_odds(vec![race], odds)
        .backtest(d(2026, 1, 1), d(2026, 1, 31), Some(0.2), EstimationConfig::default())
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
        .backtest(d(2026, 1, 1), d(2026, 1, 31), Some(0.0), EstimationConfig::default())
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
    let report = app
        .backtest(d(2026, 1, 1), d(2026, 1, 31), None, EstimationConfig::default())
        .await
        .unwrap();
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
    let report = app
        .backtest(d(2026, 1, 1), d(2026, 1, 31), None, EstimationConfig::default())
        .await
        .unwrap();

    assert_eq!(report.races_evaluated, 1);
    // 非出走馬が除外され、発走馬の最高スタッツ ウマA が 1 着 → 単勝的中
    assert!(
        (report.win_hit_rate - 1.0).abs() < 1e-9,
        "出走取消・競走除外が母集合に混入している (win_hit_rate={})",
        report.win_hit_rate
    );
}
