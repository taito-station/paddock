//! backtest interactor のオーケストレーション検証。
//!
//! in-memory モックで、確定レース 1 件に対しトップ選好馬の選択・的中突合・想定回収率が
//! 期待どおり集計されることを確認する（指標計算自体は domain 側で単体テスト済み）。

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::NaiveDate;
use paddock_domain::JockeyFormRun;
use paddock_domain::horse_result::{FinishingPosition, GateNum, HorseName, HorseNum, ResultStatus};
use paddock_domain::{
    DatedCounts, EstimationConfig, HorseResult, JockeyName, OddsValue, Race, RaceId, RaceOdds,
    RecencyConfig, RecentRun, StandardTimes, Surface, TrackCondition, TrainerName, Venue,
};
use paddock_use_case::repository::{
    CourseStatsRow, GroupStat, HorseRecencyStats, HorseStatsRow, JockeyStatsRow, OddsRepository,
    RecencySeries, StatsRepository, TrainerStatsRow,
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

/// horse_stats / horse_stats_batch の両経路から呼ぶ共通ロジック。
/// ウマA=高スタッツ / ウマS=最高スタッツ / ウマB=道悪巧者 / 他=低スタッツ。
fn make_horse_stats_row(name: &HorseName) -> HorseStatsRow {
    let win_rate = match name.value() {
        "ウマA" => 0.3,
        "ウマS" => 0.9,
        _ => 0.05,
    };
    let mut row = horse_stats_with_surface_win(win_rate);
    if name.value() == "ウマB" {
        row.by_track_condition = vec![make_group("不良", 10, 10)];
    }
    row
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
    /// horse_stats_batch の呼び出し回数カウンタ。日付単位バッチ化の効果検証に使う（#223）。
    horse_stats_batch_calls: Arc<AtomicUsize>,
}

impl StatsRepository for MockRepo {
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
    ) -> Result<Vec<RecentRun>> {
        Ok(Vec::new())
    }

    async fn find_jockey_recent_runs(
        &self,
        _jockey: &JockeyName,
        _before: NaiveDate,
        _limit: u32,
    ) -> Result<Vec<JockeyFormRun>> {
        Ok(Vec::new())
    }

    async fn standard_times(&self, _before: NaiveDate) -> Result<StandardTimes> {
        Ok(StandardTimes::default())
    }

    async fn horse_stats(
        &self,
        name: &HorseName,
        _as_of: Option<NaiveDate>,
    ) -> Result<HorseStatsRow> {
        Ok(make_horse_stats_row(name))
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

    fn horse_stats_batch(
        &self,
        names: &[HorseName],
        _as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<HashMap<HorseName, HorseStatsRow>>> + Send {
        // 呼び出し回数をカウント（日付単位バッチ化の効果検証用、#223）。
        self.horse_stats_batch_calls.fetch_add(1, Ordering::Relaxed);
        // self は async move に入れられないため、names を owned に変換して async move で処理。
        // ロジックは make_horse_stats_row に集約済み（horse_stats との二重管理を解消）。
        let names_owned = names.to_vec();
        async move {
            let mut out = HashMap::new();
            for name in &names_owned {
                out.entry(name.clone())
                    .or_insert_with(|| make_horse_stats_row(name));
            }
            Ok(out)
        }
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
}

impl OddsRepository for MockRepo {
    async fn find_race_odds(
        &self,
        race_id: &RaceId,
        _as_of: Option<NaiveDate>,
    ) -> Result<Option<RaceOdds>> {
        Ok(self.race_odds.get(race_id.value()).cloned())
    }
    async fn save_race_odds(&self, _: &paddock_use_case::repository::RaceOddsRecord) -> Result<()> {
        unimplemented!()
    }
    async fn purge_race_odds_snapshots(&self, _: NaiveDate) -> Result<u64> {
        Ok(0)
    }
    async fn count_race_odds_snapshots_before(&self, _: NaiveDate) -> Result<u64> {
        Ok(0)
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
    fn fetch_if_exists(&self, _: &str) -> Result<paddock_use_case::FetchProbe> {
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
    Interactor::new(
        MockRepo {
            races,
            race_odds,
            horse_stats_batch_calls: Arc::new(AtomicUsize::new(0)),
        },
        NullParser,
        NullFetcher,
    )
}

/// バッチ呼び出し回数を外部から観察できる interactor。日付単位バッチ化の効果検証に使う（#223）。
fn interactor_with_batch_counter(
    races: Vec<Race>,
    counter: Arc<AtomicUsize>,
) -> Interactor<MockRepo, NullParser, NullFetcher> {
    Interactor::new(
        MockRepo {
            races,
            race_odds: HashMap::new(),
            horse_stats_batch_calls: counter,
        },
        NullParser,
        NullFetcher,
    )
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
        .backtest(
            d(2026, 1, 1),
            d(2026, 1, 31),
            None,
            EstimationConfig::default(),
            false,
        )
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
        .backtest(
            d(2026, 1, 1),
            d(2026, 1, 31),
            None,
            EstimationConfig::default(),
            false,
        )
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
        .backtest(
            d(2026, 1, 1),
            d(2026, 1, 31),
            None,
            EstimationConfig::default(),
            false,
        )
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
        .backtest(
            d(2026, 1, 1),
            d(2026, 1, 31),
            None,
            EstimationConfig::default(),
            false,
        )
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
        .backtest(
            d(2026, 1, 1),
            d(2026, 1, 31),
            None,
            EstimationConfig::default(),
            false,
        )
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
        .backtest(
            d(2026, 1, 1),
            d(2026, 1, 31),
            None,
            EstimationConfig::default(),
            false,
        )
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
        recent_form_weight: None,
        trend_n: 1,
        jockey_recent_form_weight: None,
        win_power: None,
        place_show_power: None,
        impute_missing_factors: false,
    };
    let on = interactor(vec![soft_track_race(None)])
        .backtest(d(2026, 1, 1), d(2026, 1, 31), None, cfg, false)
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
        .backtest(
            d(2026, 1, 1),
            d(2026, 1, 31),
            None,
            EstimationConfig::default(),
            false,
        )
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
        .backtest(
            d(2026, 1, 1),
            d(2026, 1, 31),
            None,
            EstimationConfig::default(),
            false,
        )
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
        .backtest(
            d(2026, 1, 1),
            d(2026, 1, 31),
            None,
            EstimationConfig::default(),
            false,
        )
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
        .backtest(
            d(2026, 1, 1),
            d(2026, 1, 31),
            Some(0.2),
            EstimationConfig::default(),
            false,
        )
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
        .backtest(
            d(2026, 1, 1),
            d(2026, 1, 31),
            Some(0.2),
            EstimationConfig::default(),
            false,
        )
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
        .backtest(
            d(2026, 1, 1),
            d(2026, 1, 31),
            Some(0.0),
            EstimationConfig::default(),
            false,
        )
        .await
        .unwrap();
    assert!(
        (blended.win_hit_rate - 1.0).abs() < 1e-9,
        "fallback blended win_hit={}",
        blended.win_hit_rate
    );
}

#[tokio::test]
async fn backtest_same_day_multi_race_evaluates_independently() {
    // 同一日（2026-1-10）に 2 つのレースがある場合、日付単位バッチで統計を共有しながら
    // 各レースの評価が独立して正しく行われることを検証する（#223 日付バッチ化の回帰テスト）。
    //
    // Race A (1R): ウマA(高スタッツ, 1着) / ウマB(低スタッツ, 2着) → 本命ウマA, 単勝的中
    // Race B (5R): ウマC(低スタッツ, 1着) / ウマD(低スタッツ, 2着) → 本命は馬番小さい方=1着的中
    // ※ ウマC/ウマD は mock で win_rate=0.05 (同値)。馬番昇順タイブレーク → ウマC(horse_num=1)が本命。
    // 2レース合算: 2/2 的中 → win_hit_rate = 1.0
    //
    // 注意: horse_stats_batch 等のバッチ呼び出し回数はこのテストでは検証しない。
    // バッチ呼び出し回数の検証は backtest_date_batch_calls_horse_stats_batch_once_per_day を参照。
    // このテストは「バッチ化後も各レースが独立して正しく評価される」正当性の回帰テスト。
    let race_a = finished_race();
    let race_b = Race {
        race_id: RaceId::try_from("2026-1-nakayama-1-5R").unwrap(),
        date: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
        venue: Venue::Nakayama,
        round: 1,
        day: 1,
        race_num: 5,
        surface: Surface::Turf,
        distance: 2000,
        track_condition: None,
        weather: None,
        results: vec![
            result(1, 1, "ウマC", 1, Some(5.0)), // 低スタッツだが馬番1→本命、1着
            result(2, 5, "ウマD", 2, None),      // 低スタッツ、2着
        ],
    };

    let app = interactor(vec![race_a, race_b]);
    let report = app
        .backtest(
            d(2026, 1, 1),
            d(2026, 1, 31),
            None,
            EstimationConfig::default(),
            false,
        )
        .await
        .unwrap();

    assert_eq!(
        report.races_evaluated, 2,
        "同日2レースがそれぞれ評価されること"
    );
    // win_hit_rate=1.0 は 2/2 的中を意味する。
    // Race A: 本命ウマA(horse_num=1, win_rate=0.3) が 1 着 → 的中。
    // Race B: 本命ウマC(horse_num=1, win_rate=0.05) が 1 着 → 的中（ウマD と同率スタッツ、馬番小優先）。
    // 両レースが同日同コース設定（中山芝2000）なので course_cache ヒットも通る。
    assert!(
        (report.win_hit_rate - 1.0).abs() < 1e-9,
        "各レースの評価が独立しており、両レースで本命が的中すること (win_hit_rate={})",
        report.win_hit_rate
    );
}

#[tokio::test]
async fn backtest_date_batch_calls_horse_stats_batch_once_per_day() {
    // 日付単位バッチ化（#223）の効果: 同一日の複数レースでも horse_stats_batch は 1 日 1 回だけ
    // 呼ばれることを検証する。回帰すると「1 レース 1 回」に戻り assert が 2 になる。
    let race_a = finished_race(); // 2026-01-10, 1R
    let race_b = Race {
        race_id: RaceId::try_from("2026-1-nakayama-1-5R").unwrap(),
        date: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(), // 同日
        venue: Venue::Nakayama,
        round: 1,
        day: 1,
        race_num: 5,
        surface: Surface::Turf,
        distance: 2000,
        track_condition: None,
        weather: None,
        results: vec![
            result(1, 1, "ウマC", 1, Some(5.0)), // (horse_num, gate, name, finish, odds)
            result(2, 5, "ウマD", 2, None),
        ],
    };

    let counter = Arc::new(AtomicUsize::new(0));
    let app = interactor_with_batch_counter(vec![race_a, race_b], counter.clone());
    app.backtest(
        d(2026, 1, 1),
        d(2026, 1, 31),
        None,
        EstimationConfig::default(),
        false,
    )
    .await
    .unwrap();

    assert_eq!(
        counter.load(Ordering::Relaxed),
        1,
        "同日 2 レースでも horse_stats_batch は 1 回だけ呼ばれること（日付単位バッチ）"
    );
}

#[tokio::test]
async fn backtest_empty_when_no_races() {
    let app = interactor(Vec::new());
    let report = app
        .backtest(
            d(2026, 1, 1),
            d(2026, 1, 31),
            None,
            EstimationConfig::default(),
            false,
        )
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
        .backtest(
            d(2026, 1, 1),
            d(2026, 1, 31),
            None,
            EstimationConfig::default(),
            false,
        )
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

#[tokio::test]
async fn backtest_dump_features_none_when_not_requested() {
    // dump_features=false では feature_dump は None で既存挙動と不変（#272 Phase A）。
    let report = interactor(vec![finished_race()])
        .backtest(
            d(2026, 1, 1),
            d(2026, 1, 31),
            None,
            EstimationConfig::default(),
            false,
        )
        .await
        .unwrap();
    assert!(report.feature_dump.is_none());
}

#[tokio::test]
async fn backtest_dump_features_collects_starters_with_labels_and_market_odds() {
    // dump_features=true で全発走馬の素性＋ラベル＋当時市場単勝を収集する（#272 Phase A）。
    // race_odds に ウマA(馬番1) の単勝 7.0 を置き、win_odds が PDF 確定単勝(4.0)より市場を優先する
    // ことと、市場が無い ウマB(馬番2, PDF 単勝も None) は None になることを検証する。
    let race = finished_race();
    let mut odds = HashMap::new();
    odds.insert(
        race.race_id.value().to_string(),
        win_only_odds(race.race_id.value(), 1, 7.0),
    );
    let report = interactor_with_odds(vec![race], odds)
        .backtest(
            d(2026, 1, 1),
            d(2026, 1, 31),
            None,
            EstimationConfig::default(),
            true,
        )
        .await
        .unwrap();

    let rows = report.feature_dump.expect("dump 要求時は Some");
    // finished_race の発走馬は 2 頭のみ（非発走馬は entry_factors=starters から除外される）。
    assert_eq!(rows.len(), 2, "発走馬 1 頭につき 1 行のはず");

    let a = rows
        .iter()
        .find(|r| r.horse_num == 1)
        .expect("ウマA(馬番1) の行");
    assert_eq!(a.race_id, "2026-1-nakayama-1-1R");
    assert_eq!(a.date, d(2026, 1, 10));
    assert_eq!(a.finishing_position, Some(1));
    // 当時市場 7.0 を PDF 確定単勝 4.0 より優先。
    assert_eq!(a.win_odds, Some(7.0));
    assert_eq!(a.popularity, None);
    // ウマA は芝の出走実績があるため horse_surface は欠落しない（素性が運ばれている証跡）。
    assert!(a.factors.horse_surface.is_some());

    let b = rows
        .iter()
        .find(|r| r.horse_num == 2)
        .expect("ウマB(馬番2) の行");
    assert_eq!(b.finishing_position, Some(2));
    // 市場単勝なし・PDF 単勝も None → win_odds は None（欠落は 0 埋めしない）。
    assert_eq!(b.win_odds, None);
    // 騎手・調教師を持たない fixture なので欠落 factor が None で運ばれる（母数除外項の正例）。
    assert!(b.factors.trainer_surface.is_none());
    assert!(b.factors.jockey_surface.is_none());

    // 内蔵モデル予測（#309）が全馬に付き [0,1] に収まること。高スタッツの ウマA は低スタッツの
    // ウマB より単勝確率が高い（probs が忠実に運ばれている証跡）。
    for r in &rows {
        for p in [r.model_win, r.model_place, r.model_show] {
            assert!((0.0..=1.0).contains(&p), "確率が [0,1] 外: {p}");
        }
    }
    assert!(
        a.model_win > b.model_win,
        "高スタッツ ウマA の単勝確率が ウマB 以下 (a={}, b={})",
        a.model_win,
        b.model_win
    );
}
