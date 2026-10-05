//! Unit tests for predict_race interactor.
//!
//! Uses in-memory mocks for all repositories to test label resolution,
//! probability normalization, and the not-found error path.

use std::collections::HashMap;

use paddock_domain::horse_result::{GateNum, HorseName, HorseNum};
use paddock_domain::{
    FinishingPosition, HorseEntry, HorseResult, JockeyFormRun, JockeyName, Race, RaceCard, RaceId,
    RecentRun, ResultStatus, StandardTimes, Surface, TrackCondition, TrainerName, Venue,
};
use paddock_use_case::repository::{
    CourseStatsRow, GroupStat, HandicapNoteRow, HorseStatsRow, JockeyStatsRow, OddsRepository,
    RaceCardRepository, StatsRepository, TrainerStatsRow,
};
use paddock_use_case::{Error, Interactor, Result};

// --- helpers ----------------------------------------------------------------

fn win_of(probs: &[paddock_domain::HorseProbability], name: &str) -> f64 {
    probs
        .iter()
        .find(|p| p.horse_name.value() == name)
        .unwrap()
        .win_prob
}

fn make_group(label: &str, starts: u32, wins: u32, places: u32, shows: u32) -> GroupStat {
    GroupStat {
        label: label.to_string(),
        starts,
        wins,
        places,
        shows,
    }
}

fn make_race_card(race_id: &str) -> RaceCard {
    RaceCard {
        race_id: RaceId::try_from(race_id).unwrap(),
        date: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        post_time: None,
        venue: Venue::Tokyo,
        round: 1,
        day: 1,
        race_num: 1,
        surface: Surface::Turf,
        distance: 2000,
        race_class: None,
        race_name: None,
        entries: vec![
            HorseEntry {
                gate_num: GateNum::try_from(1u32).unwrap(),
                horse_num: HorseNum::try_from(1u32).unwrap(),
                horse_name: HorseName::try_from("ウマA").unwrap(),
                jockey: None,
                trainer: None,
                weight_carried: None,
            },
            HorseEntry {
                gate_num: GateNum::try_from(5u32).unwrap(),
                horse_num: HorseNum::try_from(2u32).unwrap(),
                horse_name: HorseName::try_from("ウマB").unwrap(),
                jockey: None,
                trainer: None,
                weight_carried: None,
            },
        ],
    }
}

fn horse_stats_with_surface_win(win_rate: f64) -> HorseStatsRow {
    let starts = 10;
    let wins = (win_rate * starts as f64).round() as u32;
    HorseStatsRow {
        horse_name: "".to_string(),
        by_surface: vec![
            make_group("芝", starts, wins, wins + 1, wins + 2),
            make_group("ダート", 5, 0, 0, 0),
        ],
        by_distance_band: vec![
            make_group("〜1400m", 0, 0, 0, 0),
            make_group("1500〜1800m", 0, 0, 0, 0),
            make_group("1900〜2200m", starts, wins, wins + 1, wins + 2),
            make_group("2300m〜", 0, 0, 0, 0),
        ],
        by_gate_group: vec![],
        by_track_condition: vec![],
        by_popularity_band: vec![],
        by_venue: vec![],
        by_jockey: vec![],
        overall: make_group("全体", starts, wins, wins + 1, wins + 2),
    }
}

fn course_stats_with_gate(inner_win: u32, middle_win: u32) -> CourseStatsRow {
    CourseStatsRow {
        venue: "東京".to_string(),
        distance: 2000,
        surface: "turf".to_string(),
        by_gate_group: vec![
            make_group("Inner (1-3)", 20, inner_win, inner_win + 2, inner_win + 4),
            make_group(
                "Middle (4-6)",
                20,
                middle_win,
                middle_win + 2,
                middle_win + 4,
            ),
            make_group("Outer (7-8)", 20, 1, 3, 5),
        ],
    }
}

// --- mock repository --------------------------------------------------------

struct MockRepo {
    card: Option<RaceCard>,
    odds: Option<paddock_domain::RaceOdds>,
    /// 馬名 → by_track_condition スタッツ（#73 のテスト用。未登録馬は空 = 馬場実績なし）。
    track_condition_stats: HashMap<String, Vec<GroupStat>>,
    /// 調教師名 → by_surface スタッツ（#74 のテスト用。未登録は空 = 実績なし）。
    trainer_surface_stats: HashMap<String, Vec<GroupStat>>,
    /// 騎手名 → by_surface スタッツ（#205 のテスト用。未登録は空 = 実績なし）。
    jockey_surface_stats: HashMap<String, Vec<GroupStat>>,
    /// 馬名 → 近走（#552 の近走被覆テスト用。未登録馬は空 = 近走なし）。
    recent_runs: HashMap<String, Vec<RecentRun>>,
    /// 統計の取得に渡された as_of（#724: 再構成の推定が全統計に as_of を渡すかの検証用）。
    as_of_seen: std::sync::Mutex<Vec<(&'static str, Option<chrono::NaiveDate>)>>,
    /// #724: 初回スイープの固定（再構成の対象レース）。
    pins: Vec<paddock_use_case::repository::LiveEvPin>,
    /// #724: race_id → (朝のスナップショット, T-40 のスナップショット)。
    snapshots: HashMap<
        String,
        (
            Option<paddock_use_case::repository::SnapshotOdds>,
            Option<paddock_use_case::repository::SnapshotOdds>,
        ),
    >,
    /// #724: find_race_odds_snapshot に渡された時点。
    points_seen: std::sync::Mutex<Vec<(String, paddock_use_case::repository::SnapshotPoint)>>,
}

impl MockRepo {
    fn see(&self, what: &'static str, as_of: Option<chrono::NaiveDate>) {
        self.as_of_seen.lock().unwrap().push((what, as_of));
    }
}

impl StatsRepository for MockRepo {
    async fn horse_stats(
        &self,
        name: &HorseName,
        as_of: Option<chrono::NaiveDate>,
    ) -> Result<HorseStatsRow> {
        self.see("horse", as_of);
        let win_rate = if name.value() == "ウマA" { 0.2 } else { 0.1 };
        let mut row = horse_stats_with_surface_win(win_rate);
        row.by_track_condition = self
            .track_condition_stats
            .get(name.value())
            .cloned()
            .unwrap_or_default();
        Ok(row)
    }
    async fn course_stats(
        &self,
        _: Venue,
        _: u32,
        _: Surface,
        as_of: Option<chrono::NaiveDate>,
    ) -> Result<CourseStatsRow> {
        self.see("course", as_of);
        Ok(course_stats_with_gate(4, 2))
    }
    /// 手動ハンデ精査材料（#628）は盤の提示専用で predict 経路は使わないため、
    /// 全馬「過去走 0 件」で応答する（`HandicapNoteRow::default()`＝該当なし。材料未取得ではない）。確率推定には入らないので predict の期待値は変わらない。
    async fn horse_handicap_notes(
        &self,
        names: &[HorseName],
        _: Venue,
        _: Surface,
        _: u32,
        _as_of: Option<chrono::NaiveDate>,
    ) -> Result<HashMap<HorseName, HandicapNoteRow>> {
        Ok(names
            .iter()
            .map(|n| (n.clone(), HandicapNoteRow::default()))
            .collect())
    }
    async fn jockey_stats(
        &self,
        name: &JockeyName,
        as_of: Option<chrono::NaiveDate>,
    ) -> Result<JockeyStatsRow> {
        self.see("jockey", as_of);
        Ok(JockeyStatsRow {
            jockey_name: name.value().to_string(),
            overall: make_group("全体", 0, 0, 0, 0),
            by_surface: self
                .jockey_surface_stats
                .get(name.value())
                .cloned()
                .unwrap_or_default(),
            by_gate_group: vec![],
            by_venue: vec![],
            by_distance_band: vec![],
        })
    }
    async fn trainer_stats(
        &self,
        name: &TrainerName,
        as_of: Option<chrono::NaiveDate>,
    ) -> Result<TrainerStatsRow> {
        self.see("trainer", as_of);
        Ok(TrainerStatsRow {
            trainer_name: name.value().to_string(),
            overall: make_group("全体", 0, 0, 0, 0),
            by_surface: self
                .trainer_surface_stats
                .get(name.value())
                .cloned()
                .unwrap_or_default(),
            by_gate_group: vec![],
        })
    }

    async fn find_finished_races_between(
        &self,
        _from: chrono::NaiveDate,
        _to: chrono::NaiveDate,
    ) -> Result<Vec<Race>> {
        Ok(Vec::new())
    }

    async fn find_recent_runs(
        &self,
        name: &HorseName,
        _before: chrono::NaiveDate,
        _limit: u32,
    ) -> Result<Vec<RecentRun>> {
        Ok(self
            .recent_runs
            .get(name.value())
            .cloned()
            .unwrap_or_default())
    }

    async fn find_jockey_recent_runs(
        &self,
        _jockey: &JockeyName,
        _before: chrono::NaiveDate,
        _limit: u32,
    ) -> Result<Vec<JockeyFormRun>> {
        Ok(Vec::new())
    }

    async fn standard_times(&self, _before: chrono::NaiveDate) -> Result<StandardTimes> {
        Ok(StandardTimes::default())
    }
}

impl RaceCardRepository for MockRepo {
    async fn save_race_card(&self, _: &RaceCard) -> Result<()> {
        unimplemented!()
    }
    async fn find_race_card(&self, _: &RaceId) -> Result<Option<RaceCard>> {
        Ok(self.card.clone())
    }
    async fn find_post_times_by_date(
        &self,
        _date: chrono::NaiveDate,
    ) -> Result<std::collections::HashMap<RaceId, chrono::NaiveTime>> {
        unimplemented!()
    }
    async fn find_race_names_by_date(
        &self,
        _date: chrono::NaiveDate,
    ) -> Result<std::collections::HashMap<RaceId, String>> {
        unimplemented!()
    }
    async fn find_race_classes_by_date(
        &self,
        _date: chrono::NaiveDate,
    ) -> Result<std::collections::HashMap<RaceId, paddock_domain::RaceClass>> {
        unimplemented!()
    }
}

impl OddsRepository for MockRepo {
    async fn save_race_odds(&self, _: &paddock_use_case::repository::RaceOddsRecord) -> Result<()> {
        unimplemented!()
    }
    async fn find_race_odds(
        &self,
        _: &RaceId,
        _: Option<chrono::NaiveDate>,
    ) -> Result<Option<paddock_domain::RaceOdds>> {
        Ok(self.odds.clone())
    }
    async fn find_race_odds_morning(
        &self,
        _: &RaceId,
    ) -> Result<Option<paddock_use_case::repository::MorningRaceOdds>> {
        Ok(None)
    }
    async fn find_race_odds_snapshot(
        &self,
        race_id: &RaceId,
        point: paddock_use_case::repository::SnapshotPoint,
    ) -> Result<Option<paddock_use_case::repository::SnapshotOdds>> {
        use paddock_use_case::repository::SnapshotPoint;
        self.points_seen
            .lock()
            .unwrap()
            .push((race_id.value().to_string(), point));
        let Some((morning, t40)) = self.snapshots.get(race_id.value()) else {
            return Ok(None);
        };
        Ok(match point {
            SnapshotPoint::FirstWinSince(_) => morning.clone(),
            SnapshotPoint::FirstCompleteSince(_) => t40.clone(),
        })
    }
    async fn purge_race_odds_snapshots(&self, _: chrono::NaiveDate) -> Result<u64> {
        Ok(0)
    }
    async fn count_race_odds_snapshots_before(&self, _: chrono::NaiveDate) -> Result<u64> {
        Ok(0)
    }
    async fn find_unpriced_bet_types(
        &self,
        _: &RaceId,
    ) -> Result<Vec<paddock_use_case::repository::UnpricedObservation>> {
        Ok(Vec::new())
    }
    async fn record_unpriced_bet_types(
        &self,
        _: &RaceId,
        _: &std::collections::BTreeSet<paddock_domain::BetType>,
        _: &std::collections::BTreeSet<paddock_domain::BetType>,
        _: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        // 記録内容を検証しないダブルなので no-op（unimplemented!() だと将来この経路を
        // 踏んだ瞬間に無関係な panic でテストが落ちる）。
        Ok(())
    }
}

impl paddock_use_case::repository::LiveEvRepository for MockRepo {
    async fn find_live_ev_by_date(
        &self,
        _: chrono::NaiveDate,
    ) -> Result<Vec<paddock_use_case::repository::LiveEvSnapshot>> {
        Ok(Vec::new())
    }
    async fn find_live_ev_pins_by_date(
        &self,
        _: chrono::NaiveDate,
    ) -> Result<Vec<paddock_use_case::repository::LiveEvPin>> {
        Ok(self.pins.clone())
    }
    async fn save_live_ev_snapshot(
        &self,
        _: &paddock_use_case::repository::LiveEvSnapshotRecord,
    ) -> Result<()> {
        Ok(())
    }
}

fn interactor(card: Option<RaceCard>) -> Interactor<MockRepo> {
    Interactor::new(MockRepo {
        card,
        odds: None,
        track_condition_stats: HashMap::new(),
        trainer_surface_stats: HashMap::new(),
        jockey_surface_stats: HashMap::new(),
        recent_runs: HashMap::new(),
        as_of_seen: Default::default(),
        pins: Vec::new(),
        snapshots: HashMap::new(),
        points_seen: Default::default(),
    })
}

fn interactor_with_odds(
    card: Option<RaceCard>,
    odds: paddock_domain::RaceOdds,
) -> Interactor<MockRepo> {
    Interactor::new(MockRepo {
        card,
        odds: Some(odds),
        track_condition_stats: HashMap::new(),
        trainer_surface_stats: HashMap::new(),
        jockey_surface_stats: HashMap::new(),
        recent_runs: HashMap::new(),
        as_of_seen: Default::default(),
        pins: Vec::new(),
        snapshots: HashMap::new(),
        points_seen: Default::default(),
    })
}

fn interactor_with_tc_stats(
    card: Option<RaceCard>,
    track_condition_stats: HashMap<String, Vec<GroupStat>>,
) -> Interactor<MockRepo> {
    Interactor::new(MockRepo {
        card,
        odds: None,
        track_condition_stats,
        trainer_surface_stats: HashMap::new(),
        jockey_surface_stats: HashMap::new(),
        recent_runs: HashMap::new(),
        as_of_seen: Default::default(),
        pins: Vec::new(),
        snapshots: HashMap::new(),
        points_seen: Default::default(),
    })
}

fn interactor_with_trainer_stats(
    card: Option<RaceCard>,
    trainer_surface_stats: HashMap<String, Vec<GroupStat>>,
) -> Interactor<MockRepo> {
    Interactor::new(MockRepo {
        card,
        odds: None,
        track_condition_stats: HashMap::new(),
        trainer_surface_stats,
        jockey_surface_stats: HashMap::new(),
        recent_runs: HashMap::new(),
        as_of_seen: Default::default(),
        pins: Vec::new(),
        snapshots: HashMap::new(),
        points_seen: Default::default(),
    })
}

fn interactor_with_jockey_stats(
    card: Option<RaceCard>,
    jockey_surface_stats: HashMap<String, Vec<GroupStat>>,
) -> Interactor<MockRepo> {
    Interactor::new(MockRepo {
        card,
        odds: None,
        track_condition_stats: HashMap::new(),
        trainer_surface_stats: HashMap::new(),
        jockey_surface_stats,
        recent_runs: HashMap::new(),
        as_of_seen: Default::default(),
        pins: Vec::new(),
        snapshots: HashMap::new(),
        points_seen: Default::default(),
    })
}

/// #552: 一部の馬にだけ近走を持たせた MockRepo を組む（近走被覆の部分カウント検証用）。
fn interactor_with_recent_runs(
    card: Option<RaceCard>,
    recent_runs: HashMap<String, Vec<RecentRun>>,
) -> Interactor<MockRepo> {
    Interactor::new(MockRepo {
        card,
        odds: None,
        track_condition_stats: HashMap::new(),
        trainer_surface_stats: HashMap::new(),
        jockey_surface_stats: HashMap::new(),
        recent_runs,
        as_of_seen: Default::default(),
        pins: Vec::new(),
        snapshots: HashMap::new(),
        points_seen: Default::default(),
    })
}

/// #552: 近走被覆テスト用の最小 RecentRun（中身は被覆カウントに無関係。非空であることだけが要点）。
fn sample_recent_run() -> RecentRun {
    RecentRun {
        date: chrono::NaiveDate::from_ymd_opt(2025, 12, 1).unwrap(),
        surface: Surface::Turf,
        distance: 1600,
        result: HorseResult {
            finishing_position: Some(FinishingPosition::try_from(1u32).unwrap()),
            status: ResultStatus::Finished,
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(1u32).unwrap(),
            horse_name: HorseName::try_from("ウマA").unwrap(),
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
        },
        corner_positions: None,
        field_size: None,
    }
}

// --- tests ------------------------------------------------------------------

#[tokio::test]
async fn predict_race_returns_not_found_when_card_missing() {
    let app = interactor(None);
    let race_id = RaceId::try_from("2026-1-tokyo-1-R1").unwrap();
    let err = app.predict_race(&race_id, None, None).await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)));
}

#[tokio::test]
async fn predict_race_win_sums_to_one_and_monotone() {
    let card = make_race_card("2026-1-tokyo-1-R1");
    let app = interactor(Some(card));
    let race_id = RaceId::try_from("2026-1-tokyo-1-R1").unwrap();
    let probs = app.predict_race(&race_id, None, None).await.unwrap();

    assert_eq!(probs.len(), 2);

    // win は 1 着＝1 ポジションなので合計 ≒ 1.0。place(→2.0)/show(→3.0) は 2 頭立てでは
    // 上限 1.0 クランプが効くため、各値の範囲と単調性を確認する（ADR 0007）。
    let win_total: f64 = probs.iter().map(|p| p.win_prob).sum();
    assert!((win_total - 1.0).abs() < 1e-10, "win sum={win_total}");
    for p in &probs {
        assert!((0.0..=1.0).contains(&p.win_prob));
        assert!((0.0..=1.0).contains(&p.place_prob));
        assert!((0.0..=1.0).contains(&p.show_prob));
        assert!(
            p.win_prob <= p.place_prob && p.place_prob <= p.show_prob,
            "non-monotonic: {p:?}"
        );
    }
}

#[tokio::test]
async fn predict_race_views_with_odds_none_when_no_odds() {
    // オッズ未取得（find_race_odds → None）なら odds は None＝過去データ視点のみ出せる（#272 ③④）。
    let card = make_race_card("2026-1-tokyo-1-R1");
    let app = interactor(Some(card)); // odds: None
    let race_id = RaceId::try_from("2026-1-tokyo-1-R1").unwrap();
    let (views, odds) = app
        .predict_race_views_with_odds(
            &race_id,
            None,
            None,
            false,
            &paddock_domain::EstimationConfig::production(),
        )
        .await
        .unwrap();
    assert_eq!(views.pure.len(), 2);
    assert!(odds.is_none(), "オッズ未取得なら odds は None");
}

#[tokio::test]
async fn predict_race_views_with_odds_returns_odds_and_diagnostics() {
    // オッズありなら odds Some。呼び出し側が pair_ev_diagnostics で (軸, 各ペア行) を得る
    // （軸=blended・EV=pure の循環断ち #272。旧 predict_race_with_diagnostics の後継検証）。
    let race_id_str = "2026-1-tokyo-1-R1";
    let card = make_race_card(race_id_str);
    let mut odds = paddock_domain::RaceOdds::empty(RaceId::try_from(race_id_str).unwrap());
    odds.quinella.insert(
        paddock_domain::Pair::try_from((
            HorseNum::try_from(1u32).unwrap(),
            HorseNum::try_from(2u32).unwrap(),
        ))
        .unwrap(),
        paddock_domain::OddsValue::try_from((paddock_domain::BetType::Quinella, 5.0)).unwrap(),
    );
    let app = interactor_with_odds(Some(card), odds);
    let race_id = RaceId::try_from(race_id_str).unwrap();
    let (views, odds) = app
        .predict_race_views_with_odds(
            &race_id,
            None,
            None,
            false,
            &paddock_domain::EstimationConfig::production(),
        )
        .await
        .unwrap();
    let odds = odds.expect("オッズありなら odds は Some");
    let diag = paddock_domain::pair_ev_diagnostics(&views.blended, &views.pure, &odds, 5);
    assert!(diag.axis.is_some(), "軸が決まる");
    assert_eq!(diag.rows.len(), 1, "2 頭立て → 相手 1 頭");
}

#[tokio::test]
async fn predict_race_higher_stats_horse_gets_higher_win_prob() {
    // ウマA（枠番1=Inner, win_rate=0.2）と ウマB（枠番5=Middle, win_rate=0.1）で、
    // course_stats は inner_win=4 > middle_win=2 と設定。
    // 馬スタッツ差 + コース枠番差の両方がウマA有利に働く複合テスト（意図的）。
    let card = make_race_card("2026-1-tokyo-1-R1");
    let app = interactor(Some(card));
    let race_id = RaceId::try_from("2026-1-tokyo-1-R1").unwrap();
    let probs = app.predict_race(&race_id, None, None).await.unwrap();

    let uma_a = probs
        .iter()
        .find(|p| p.horse_name.value() == "ウマA")
        .unwrap();
    let uma_b = probs
        .iter()
        .find(|p| p.horse_name.value() == "ウマB")
        .unwrap();
    assert!(
        uma_a.win_prob > uma_b.win_prob,
        "ウマA(win_rate=0.2, Inner gate) should outrank ウマB(win_rate=0.1, Middle gate)"
    );
}

#[tokio::test]
async fn predict_race_track_condition_lifts_horse_with_strong_record() {
    // ウマB だけ「良」での好成績を持つ。track_condition 未指定では馬場項は使われず、
    // Some(良) を渡すと ウマB の win_prob が上がり、相対的に ウマA は下がる（#73）。
    let card = make_race_card("2026-1-tokyo-1-R1");
    let race_id = RaceId::try_from("2026-1-tokyo-1-R1").unwrap();
    let tc: HashMap<String, Vec<GroupStat>> =
        HashMap::from([("ウマB".to_string(), vec![make_group("良", 10, 8, 9, 10)])]);

    let without = interactor_with_tc_stats(Some(card.clone()), tc.clone())
        .predict_race(&race_id, None, None)
        .await
        .unwrap();
    let with_tc = interactor_with_tc_stats(Some(card), tc)
        .predict_race(&race_id, None, Some(TrackCondition::Firm))
        .await
        .unwrap();

    assert!(
        win_of(&with_tc, "ウマB") > win_of(&without, "ウマB"),
        "良馬場巧者の ウマB は馬場項で win_prob が上がるはず: without={}, with={}",
        win_of(&without, "ウマB"),
        win_of(&with_tc, "ウマB")
    );
    assert!(win_of(&with_tc, "ウマA") < win_of(&without, "ウマA"));
    // 馬場項を加えても単調性を維持。
    for p in &with_tc {
        assert!(
            p.win_prob <= p.place_prob && p.place_prob <= p.show_prob,
            "{p:?}"
        );
    }
}

#[tokio::test]
async fn predict_race_track_condition_zero_starts_treated_as_missing() {
    // 「良」のグループはあるが出走 0 件 → 実績なしとして項ごと母数から除外され、
    // by_track_condition が空の場合と完全に一致する（0 レート扱いで減点しない、#73）。
    let card = make_race_card("2026-1-tokyo-1-R1");
    let race_id = RaceId::try_from("2026-1-tokyo-1-R1").unwrap();
    let zero_starts: HashMap<String, Vec<GroupStat>> =
        HashMap::from([("ウマA".to_string(), vec![make_group("良", 0, 0, 0, 0)])]);

    let with_zero = interactor_with_tc_stats(Some(card.clone()), zero_starts)
        .predict_race(&race_id, None, Some(TrackCondition::Firm))
        .await
        .unwrap();
    let with_empty = interactor_with_tc_stats(Some(card), HashMap::new())
        .predict_race(&race_id, None, Some(TrackCondition::Firm))
        .await
        .unwrap();

    // zip は短い方で打ち切られるため、空 vec 同士の空振り pass を先に弾く。
    assert_eq!(with_zero.len(), 2);
    assert_eq!(with_zero.len(), with_empty.len());
    for (a, b) in with_zero.iter().zip(&with_empty) {
        assert!((a.win_prob - b.win_prob).abs() < 1e-12, "{a:?} vs {b:?}");
        assert!((a.place_prob - b.place_prob).abs() < 1e-12);
        assert!((a.show_prob - b.show_prob).abs() < 1e-12);
    }
}

#[tokio::test]
async fn predict_race_blends_market_odds_when_alpha_given() {
    // モデルは ウマA 有利だが、市場は ウマB を圧倒的人気（低オッズ）にする。
    // α=0.3（市場重み 0.7）でブレンドすると ウマB の win_prob がモデルのみより上がる。
    let race_id_str = "2026-1-tokyo-1-R1";
    let card = make_race_card(race_id_str);
    let mut odds = paddock_domain::RaceOdds::empty(RaceId::try_from(race_id_str).unwrap());
    odds.win.insert(
        HorseNum::try_from(1u32).unwrap(),
        paddock_domain::OddsValue::try_from((paddock_domain::BetType::Win, 8.0)).unwrap(), // ウマA: 人気薄
    );
    odds.win.insert(
        HorseNum::try_from(2u32).unwrap(),
        paddock_domain::OddsValue::try_from((paddock_domain::BetType::Win, 1.3)).unwrap(), // ウマB: 圧倒的人気
    );
    let race_id = RaceId::try_from(race_id_str).unwrap();

    let model_only = interactor(Some(card.clone()))
        .predict_race(&race_id, None, None)
        .await
        .unwrap();
    let blended = interactor_with_odds(Some(card), odds)
        .predict_race(&race_id, Some(0.3), None)
        .await
        .unwrap();

    let b_model = model_only
        .iter()
        .find(|p| p.horse_name.value() == "ウマB")
        .unwrap()
        .win_prob;
    let b_blend = blended
        .iter()
        .find(|p| p.horse_name.value() == "ウマB")
        .unwrap()
        .win_prob;
    assert!(
        b_blend > b_model,
        "市場で圧倒的人気の ウマB はブレンドで win_prob が上がるはず: model={b_model}, blend={b_blend}"
    );
    // ブレンド後も win 合計 ≒ 1.0 と単調性を維持。
    let win_total: f64 = blended.iter().map(|p| p.win_prob).sum();
    assert!((win_total - 1.0).abs() < 1e-9, "win sum={win_total}");
    for p in &blended {
        assert!(
            p.win_prob <= p.place_prob && p.place_prob <= p.show_prob,
            "{p:?}"
        );
    }
}

#[tokio::test]
async fn predict_race_views_separates_blended_and_pure() {
    // #272 確率分離: blended は市場ブレンド（α<1.0）、pure は α=1.0（市場非依存）。市場で ウマB を
    // 圧倒的人気にすると blended の ウマB win は pure より上がる（pure は市場で動かない）。
    let race_id_str = "2026-1-tokyo-1-R1";
    let card = make_race_card(race_id_str);
    let mut odds = paddock_domain::RaceOdds::empty(RaceId::try_from(race_id_str).unwrap());
    odds.win.insert(
        HorseNum::try_from(1u32).unwrap(),
        paddock_domain::OddsValue::try_from((paddock_domain::BetType::Win, 8.0)).unwrap(), // ウマA: 人気薄
    );
    odds.win.insert(
        HorseNum::try_from(2u32).unwrap(),
        paddock_domain::OddsValue::try_from((paddock_domain::BetType::Win, 1.3)).unwrap(), // ウマB: 圧倒的人気
    );
    let rid = RaceId::try_from(race_id_str).unwrap();

    let views = interactor_with_odds(Some(card), odds)
        .predict_race_views(&rid, Some(0.3), None, true)
        .await
        .unwrap();

    // 市場で人気の ウマB は blended でのみ持ち上がり、pure（市場非依存）は据え置き。
    assert!(
        win_of(&views.blended, "ウマB") > win_of(&views.pure, "ウマB"),
        "市場ブレンドは blended のみに効く: blend={}, pure={}",
        win_of(&views.blended, "ウマB"),
        win_of(&views.pure, "ウマB")
    );
    // pure は α=1.0 で win 合計 ≒ 1.0・単調性を維持。
    let pure_total: f64 = views.pure.iter().map(|p| p.win_prob).sum();
    assert!((pure_total - 1.0).abs() < 1e-9, "pure win sum={pure_total}");
    // with_explanation=true → 根拠が全馬ぶん返る（pure と同頭数）。
    assert_eq!(views.explanations.len(), views.pure.len());
}

#[tokio::test]
async fn predict_race_views_omits_explanations_when_flag_false() {
    // with_explanation=false なら根拠は組まない（空 Vec）。確率は両系統とも返る。
    let race_id_str = "2026-1-tokyo-1-R1";
    let card = make_race_card(race_id_str);
    let app = interactor(Some(card)); // odds None でも pure/blended は出る（odds 無ければブレンド素通り）
    let rid = RaceId::try_from(race_id_str).unwrap();
    let views = app
        .predict_race_views(&rid, Some(0.2), None, false)
        .await
        .unwrap();
    assert!(
        views.explanations.is_empty(),
        "with_explanation=false なら根拠は空"
    );
    assert_eq!(views.blended.len(), 2);
    assert_eq!(views.pure.len(), 2);
}

#[tokio::test]
async fn predict_race_views_reports_recent_runs_coverage_all_missing() {
    // #552: MockRepo は find_recent_runs が常に空 = 全馬が近走ゼロ（新馬戦相当）。
    // views.recent_runs_coverage が出走頭数と「近走あり頭数=0」を正しく集計することを検証する
    // （出力側はこの集計値で信頼性低の警告を出す）。
    let race_id_str = "2026-1-tokyo-1-R1";
    let card = make_race_card(race_id_str);
    let rid = RaceId::try_from(race_id_str).unwrap();
    let views = interactor(Some(card))
        .predict_race_views(&rid, Some(0.2), None, false)
        .await
        .unwrap();
    assert_eq!(views.recent_runs_coverage.field_size, 2);
    assert_eq!(views.recent_runs_coverage.horses_with_runs, 0);
}

#[tokio::test]
async fn predict_race_views_counts_partial_recent_runs_coverage() {
    // #552: 一部の馬（ウマA）だけ近走あり → horses_with_runs=1 / field_size=2。
    // horses_with_runs の加算経路（近走ありでインクリメント）を回帰検知する。
    let race_id_str = "2026-1-tokyo-1-R1";
    let card = make_race_card(race_id_str);
    let mut runs = HashMap::new();
    runs.insert("ウマA".to_string(), vec![sample_recent_run()]);
    let rid = RaceId::try_from(race_id_str).unwrap();
    let views = interactor_with_recent_runs(Some(card), runs)
        .predict_race_views(&rid, Some(0.2), None, false)
        .await
        .unwrap();
    assert_eq!(views.recent_runs_coverage.field_size, 2);
    assert_eq!(views.recent_runs_coverage.horses_with_runs, 1);
}

#[tokio::test]
async fn predict_race_trainer_lifts_horse_with_strong_record() {
    // ウマB だけ調教師（出馬表由来の entry.trainer）に芝の好成績を持たせる。trainer 統計が
    // 無い場合（実績なし）と比べて ウマB の win_prob が上がり、ウマA は相対的に下がる（#74）。
    let race_id = "2026-1-tokyo-1-R1";
    let mut card = make_race_card(race_id);
    card.entries[1].trainer = Some(TrainerName::try_from("名伯楽").unwrap());
    let rid = RaceId::try_from(race_id).unwrap();
    let tr: HashMap<String, Vec<GroupStat>> =
        HashMap::from([("名伯楽".to_string(), vec![make_group("芝", 10, 8, 9, 10)])]);

    let without = interactor_with_trainer_stats(Some(card.clone()), HashMap::new())
        .predict_race(&rid, None, None)
        .await
        .unwrap();
    let with_tr = interactor_with_trainer_stats(Some(card), tr)
        .predict_race(&rid, None, None)
        .await
        .unwrap();

    assert!(
        win_of(&with_tr, "ウマB") > win_of(&without, "ウマB"),
        "強い調教師の ウマB は trainer 項で win_prob が上がるはず: without={}, with={}",
        win_of(&without, "ウマB"),
        win_of(&with_tr, "ウマB")
    );
    assert!(win_of(&with_tr, "ウマA") < win_of(&without, "ウマA"));
    for p in &with_tr {
        assert!(
            p.win_prob <= p.place_prob && p.place_prob <= p.show_prob,
            "{p:?}"
        );
    }
}

#[tokio::test]
async fn predict_race_jockey_lifts_horse_with_strong_record() {
    // ウマB だけ騎手（出馬表由来の entry.jockey）に芝の好成績を持たせる。騎手あり・実績なし
    // （by_surface が空）の場合と比べて ウマB の win_prob が上がり、ウマA は相対的に下がる（#205）。
    let race_id = "2026-1-tokyo-1-R1";
    let mut card = make_race_card(race_id);
    card.entries[1].jockey = Some(JockeyName::try_from("名手").unwrap());
    let rid = RaceId::try_from(race_id).unwrap();
    let jk: HashMap<String, Vec<GroupStat>> =
        HashMap::from([("名手".to_string(), vec![make_group("芝", 10, 8, 9, 10)])]);

    let without = interactor_with_jockey_stats(Some(card.clone()), HashMap::new())
        .predict_race(&rid, None, None)
        .await
        .unwrap();
    let with_jk = interactor_with_jockey_stats(Some(card), jk)
        .predict_race(&rid, None, None)
        .await
        .unwrap();

    assert!(
        win_of(&with_jk, "ウマB") > win_of(&without, "ウマB"),
        "強い騎手の ウマB は jockey 項で win_prob が上がるはず: without={}, with={}",
        win_of(&without, "ウマB"),
        win_of(&with_jk, "ウマB")
    );
    assert!(win_of(&with_jk, "ウマA") < win_of(&without, "ウマA"));
    for p in &with_jk {
        assert!(
            p.win_prob <= p.place_prob && p.place_prob <= p.show_prob,
            "{p:?}"
        );
    }
}

#[tokio::test]
async fn predict_race_jockey_zero_stats_same_as_absent() {
    // jockey=Some だが by_surface が空の馬は jockey_map に Some(empty_stats) として登録され、
    // win_prob は jockey=None（entry 自体に騎手なし）の馬と一致する（ADR 0007: 欠落は母数除外）。
    let race_id = "2026-1-tokyo-1-R1";
    let rid = RaceId::try_from(race_id).unwrap();

    let card_none = make_race_card(race_id); // ウマB: jockey=None
    let mut card_some = make_race_card(race_id);
    card_some.entries[1].jockey = Some(JockeyName::try_from("名手").unwrap()); // ウマB: jockey=Some, 実績なし

    let without = interactor(Some(card_none))
        .predict_race(&rid, None, None)
        .await
        .unwrap();
    let with_empty = interactor_with_jockey_stats(Some(card_some), HashMap::new())
        .predict_race(&rid, None, None)
        .await
        .unwrap();

    let place_of = |probs: &[paddock_domain::HorseProbability], name: &str| {
        probs
            .iter()
            .find(|p| p.horse_name.value() == name)
            .unwrap()
            .place_prob
    };
    let show_of = |probs: &[paddock_domain::HorseProbability], name: &str| {
        probs
            .iter()
            .find(|p| p.horse_name.value() == name)
            .unwrap()
            .show_prob
    };
    for name in &["ウマA", "ウマB"] {
        assert!(
            (win_of(&without, name) - win_of(&with_empty, name)).abs() < 1e-12,
            "{name}: win_prob: without={}, with_empty={}",
            win_of(&without, name),
            win_of(&with_empty, name)
        );
        assert!(
            (place_of(&without, name) - place_of(&with_empty, name)).abs() < 1e-12,
            "{name}: place_prob mismatch"
        );
        assert!(
            (show_of(&without, name) - show_of(&with_empty, name)).abs() < 1e-12,
            "{name}: show_prob mismatch"
        );
    }
}

#[tokio::test]
async fn predict_race_jockey_absent_not_penalized() {
    // 出馬表に騎手が無い（entry.jockey=None）馬は jockey 項なし。jockey_surface_stats を
    // 渡しても entry.jockey=None なら名前収集段階でスキップされ batch にも渡らない（#205）。
    let race_id = "2026-1-tokyo-1-R1";
    let rid = RaceId::try_from(race_id).unwrap();
    let baseline = interactor(Some(make_race_card(race_id)))
        .predict_race(&rid, None, None)
        .await
        .unwrap();
    let jk: HashMap<String, Vec<GroupStat>> =
        HashMap::from([("名手".to_string(), vec![make_group("芝", 10, 8, 9, 10)])]);
    let with_stats = interactor_with_jockey_stats(Some(make_race_card(race_id)), jk)
        .predict_race(&rid, None, None)
        .await
        .unwrap();

    assert_eq!(baseline.len(), 2);
    assert_eq!(baseline.len(), with_stats.len());
    for (a, b) in baseline.iter().zip(&with_stats) {
        assert!((a.win_prob - b.win_prob).abs() < 1e-12, "{a:?} vs {b:?}");
        assert!(
            (a.place_prob - b.place_prob).abs() < 1e-12,
            "{a:?} vs {b:?}"
        );
        assert!((a.show_prob - b.show_prob).abs() < 1e-12, "{a:?} vs {b:?}");
    }
}

#[tokio::test]
async fn predict_race_trainer_absent_not_penalized() {
    // 出馬表に調教師が無い（entry.trainer=None）馬は trainer 項なし。trainer_surface_stats を
    // 渡しても entry.trainer=None なら無視され、trainer 統計を一切持たない場合と一致する（#74）。
    let race_id = "2026-1-tokyo-1-R1";
    let rid = RaceId::try_from(race_id).unwrap();
    let baseline = interactor(Some(make_race_card(race_id)))
        .predict_race(&rid, None, None)
        .await
        .unwrap();
    let tr: HashMap<String, Vec<GroupStat>> =
        HashMap::from([("名伯楽".to_string(), vec![make_group("芝", 10, 8, 9, 10)])]);
    let with_stats = interactor_with_trainer_stats(Some(make_race_card(race_id)), tr)
        .predict_race(&rid, None, None)
        .await
        .unwrap();

    assert_eq!(baseline.len(), 2);
    assert_eq!(baseline.len(), with_stats.len());
    for (a, b) in baseline.iter().zip(&with_stats) {
        assert!((a.win_prob - b.win_prob).abs() < 1e-12, "{a:?} vs {b:?}");
    }
}

// --- #724: 過去の時点の買い目を再構成するための推定（predict_race_views_at） ---------------

/// 3 頭立て・騎手と調教師つきのカード（統計の取得すべてに as_of が渡るかを見るため）。
fn make_reconstruct_card(race_id: &str) -> RaceCard {
    let mut card = make_race_card(race_id);
    card.entries.push(HorseEntry {
        gate_num: GateNum::try_from(8u32).unwrap(),
        horse_num: HorseNum::try_from(3u32).unwrap(),
        horse_name: HorseName::try_from("ウマC").unwrap(),
        jockey: None,
        trainer: None,
        weight_carried: None,
    });
    for e in &mut card.entries {
        e.jockey = Some(JockeyName::try_from("騎手A").unwrap());
        e.trainer = Some(TrainerName::try_from("調教師A").unwrap());
    }
    card
}

fn win_odds(race_id: &str, pairs: &[(u32, f64)]) -> paddock_domain::RaceOdds {
    let mut odds = paddock_domain::RaceOdds::empty(RaceId::try_from(race_id).unwrap());
    for (n, v) in pairs {
        odds.win.insert(
            HorseNum::try_from(*n).unwrap(),
            paddock_domain::OddsValue::try_from((paddock_domain::BetType::Win, *v)).unwrap(),
        );
    }
    odds
}

#[tokio::test]
async fn views_at_passes_as_of_to_every_stats_lookup() {
    // 当日以降の結果が統計に混ざらないよう、馬・騎手・調教師・コースの統計すべてに as_of を渡す。
    let rid = "2026-1-tokyo-1-R1";
    let app = interactor(Some(make_reconstruct_card(rid)));
    let as_of = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let market = win_odds(rid, &[(1, 3.0), (2, 4.0), (3, 6.0)]);
    app.predict_race_views_at(
        &RaceId::try_from(rid).unwrap(),
        Some(0.2),
        &market,
        as_of,
        &paddock_domain::EstimationConfig::production(),
    )
    .await
    .unwrap();
    let seen = app.repository.as_of_seen.lock().unwrap().clone();
    for what in ["horse", "jockey", "trainer", "course"] {
        assert!(
            seen.iter().any(|(w, _)| *w == what),
            "{what} の統計を引いていない: {seen:?}"
        );
    }
    assert!(
        seen.iter().all(|(_, a)| *a == Some(as_of)),
        "すべての統計に as_of を渡す: {seen:?}"
    );
}

#[tokio::test]
async fn production_views_keep_full_period_stats() {
    // 本番の経路（predict_race_views）は従来どおり as_of なし（全期間）のまま。
    let rid = "2026-1-tokyo-1-R1";
    let app = interactor(Some(make_reconstruct_card(rid)));
    app.predict_race_views(&RaceId::try_from(rid).unwrap(), Some(0.2), None, false)
        .await
        .unwrap();
    let seen = app.repository.as_of_seen.lock().unwrap().clone();
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|(_, a)| a.is_none()), "{seen:?}");
}

#[tokio::test]
async fn views_at_limits_field_to_horses_in_the_market_snapshot() {
    // その時点の単勝に載っていない馬（後で出馬表に残っていても）は母集合に入れない。
    let rid = "2026-1-tokyo-1-R1";
    let app = interactor(Some(make_reconstruct_card(rid)));
    let market = win_odds(rid, &[(1, 2.0), (3, 3.0)]);
    let views = app
        .predict_race_views_at(
            &RaceId::try_from(rid).unwrap(),
            Some(0.2),
            &market,
            chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            &paddock_domain::EstimationConfig::production(),
        )
        .await
        .unwrap();
    let nums = |v: &[paddock_domain::HorseProbability]| -> Vec<u32> {
        v.iter().map(|p| p.horse_num.value()).collect()
    };
    assert_eq!(nums(&views.blended), vec![1, 3]);
    assert_eq!(nums(&views.pure), vec![1, 3]);
}

#[tokio::test]
async fn views_at_blends_with_the_given_market_not_the_stored_odds() {
    // リポジトリには保存オッズが無い（odds: None）。渡した市場オッズでブレンドされていれば、
    // 圧倒的人気のウマB の勝率は純モデルより上がる。
    let rid = "2026-1-tokyo-1-R1";
    let app = interactor(Some(make_reconstruct_card(rid)));
    let market = win_odds(rid, &[(1, 30.0), (2, 1.2), (3, 30.0)]);
    let views = app
        .predict_race_views_at(
            &RaceId::try_from(rid).unwrap(),
            Some(0.2),
            &market,
            chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            &paddock_domain::EstimationConfig::production(),
        )
        .await
        .unwrap();
    let b = |v: &[paddock_domain::HorseProbability]| win_of(v, "ウマB");
    assert!(
        b(&views.blended) > b(&views.pure) + 0.2,
        "blended={} pure={}",
        b(&views.blended),
        b(&views.pure)
    );
}

#[tokio::test]
async fn reconstruct_wires_time_points_as_of_and_skip_reasons() {
    use paddock_use_case::repository::{LiveEvPin, SnapshotOdds, SnapshotPoint};
    let rid = "2026-1-tokyo-1-R1";
    let date = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let pin = |race_id: &str| LiveEvPin {
        race_id: race_id.to_string(),
        axis: 1,
        partners: vec![2, 3],
        konsen_band: vec![],
        captured_at: "2026-01-01T03:00:00Z".to_string(),
    };
    // 朝は 1 番、T-40 は 2 番が圧倒的人気（どちらの盤で組んだかを軸で見分ける）
    let snap = |pairs: &[(u32, f64)], at: &str| SnapshotOdds {
        odds: win_odds(rid, pairs),
        fetched_at: at.to_string(),
    };
    let mut repo = MockRepo {
        card: Some(make_reconstruct_card(rid)),
        odds: None,
        track_condition_stats: HashMap::new(),
        trainer_surface_stats: HashMap::new(),
        jockey_surface_stats: HashMap::new(),
        recent_runs: HashMap::new(),
        as_of_seen: Default::default(),
        pins: vec![pin(rid), pin("2026-1-tokyo-1-R2")],
        snapshots: HashMap::new(),
        points_seen: Default::default(),
    };
    repo.snapshots.insert(
        rid.to_string(),
        (
            Some(snap(&[(1, 1.2), (2, 30.0), (3, 30.0)], "morning")),
            Some(snap(&[(1, 30.0), (2, 1.2), (3, 30.0)], "t40")),
        ),
    );
    let app = Interactor::new(repo);
    let out = app
        .reconstruct_race_slips(date, Some(0.2), 5000)
        .await
        .unwrap();

    // スナップショットが無い R2 は理由付きで飛ばし、R1 は組める（1 レースの失敗で止めない）
    assert_eq!(out.len(), 2);
    let r1 = out.iter().find(|r| r.recorded.race_id == rid).unwrap();
    let r2 = out.iter().find(|r| r.recorded.race_id != rid).unwrap();
    assert!(r2.slips.is_none());
    assert!(
        r2.skip
            .as_deref()
            .is_some_and(|s| s.contains("朝のスナップショットが無い"))
    );
    assert!(r1.skip.is_none());
    assert_eq!(r1.morning_at.as_deref(), Some("morning"));
    assert_eq!(r1.t40_at.as_deref(), Some("t40"));
    let slips = r1.slips.as_ref().unwrap();
    assert_eq!(
        slips.morning_pick.axis.map(|h| h.value()),
        Some(1),
        "朝の盤で選ぶ"
    );
    assert_eq!(
        slips.t40.axis.map(|h| h.value()),
        Some(2),
        "T-40 の盤で選ぶ"
    );

    // 朝 = 当日 JST 0 時以降の最初の単勝、T-40 = 初回スイープ以降の最初の全券種
    let seen = app.repository.points_seen.lock().unwrap().clone();
    let jst_midnight = chrono::DateTime::parse_from_rfc3339("2025-12-31T15:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let captured = chrono::DateTime::parse_from_rfc3339("2026-01-01T03:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert!(seen.contains(&(rid.to_string(), SnapshotPoint::FirstWinSince(jst_midnight))));
    assert!(seen.contains(&(rid.to_string(), SnapshotPoint::FirstCompleteSince(captured))));
    // 統計の as_of は開催日
    let as_of = app.repository.as_of_seen.lock().unwrap().clone();
    assert!(
        !as_of.is_empty() && as_of.iter().all(|(_, a)| *a == Some(date)),
        "{as_of:?}"
    );
}
