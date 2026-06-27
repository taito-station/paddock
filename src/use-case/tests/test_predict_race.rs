//! Unit tests for predict_race interactor.
//!
//! Uses in-memory mocks for all repositories to test label resolution,
//! probability normalization, and the not-found error path.

use std::collections::HashMap;

use paddock_domain::horse_result::{GateNum, HorseName, HorseNum};
use paddock_domain::{
    HorseEntry, JockeyFormRun, JockeyName, Race, RaceCard, RaceId, RecentRun, StandardTimes,
    Surface, TrackCondition, TrainerName, Venue,
};
use paddock_use_case::repository::{
    CourseStatsRow, GroupStat, HorseStatsRow, JockeyStatsRow, OddsRepository, RaceCardRepository,
    StatsRepository, TrainerStatsRow,
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
}

impl StatsRepository for MockRepo {
    async fn horse_stats(
        &self,
        name: &HorseName,
        _as_of: Option<chrono::NaiveDate>,
    ) -> Result<HorseStatsRow> {
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
        _as_of: Option<chrono::NaiveDate>,
    ) -> Result<CourseStatsRow> {
        Ok(course_stats_with_gate(4, 2))
    }
    async fn jockey_stats(
        &self,
        name: &JockeyName,
        _as_of: Option<chrono::NaiveDate>,
    ) -> Result<JockeyStatsRow> {
        Ok(JockeyStatsRow {
            jockey_name: name.value().to_string(),
            overall: make_group("全体", 0, 0, 0, 0),
            by_surface: self
                .jockey_surface_stats
                .get(name.value())
                .cloned()
                .unwrap_or_default(),
            by_gate_group: vec![],
        })
    }
    async fn trainer_stats(
        &self,
        name: &TrainerName,
        _as_of: Option<chrono::NaiveDate>,
    ) -> Result<TrainerStatsRow> {
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
        _name: &HorseName,
        _before: chrono::NaiveDate,
        _limit: u32,
    ) -> Result<Vec<RecentRun>> {
        Ok(Vec::new())
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
    async fn purge_race_odds_snapshots(&self, _: chrono::NaiveDate) -> Result<u64> {
        Ok(0)
    }
    async fn count_race_odds_snapshots_before(&self, _: chrono::NaiveDate) -> Result<u64> {
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

fn interactor(card: Option<RaceCard>) -> Interactor<MockRepo, NullParser, NullFetcher> {
    Interactor::new(
        MockRepo {
            card,
            odds: None,
            track_condition_stats: HashMap::new(),
            trainer_surface_stats: HashMap::new(),
            jockey_surface_stats: HashMap::new(),
        },
        NullParser,
        NullFetcher,
    )
}

fn interactor_with_odds(
    card: Option<RaceCard>,
    odds: paddock_domain::RaceOdds,
) -> Interactor<MockRepo, NullParser, NullFetcher> {
    Interactor::new(
        MockRepo {
            card,
            odds: Some(odds),
            track_condition_stats: HashMap::new(),
            trainer_surface_stats: HashMap::new(),
            jockey_surface_stats: HashMap::new(),
        },
        NullParser,
        NullFetcher,
    )
}

fn interactor_with_tc_stats(
    card: Option<RaceCard>,
    track_condition_stats: HashMap<String, Vec<GroupStat>>,
) -> Interactor<MockRepo, NullParser, NullFetcher> {
    Interactor::new(
        MockRepo {
            card,
            odds: None,
            track_condition_stats,
            trainer_surface_stats: HashMap::new(),
            jockey_surface_stats: HashMap::new(),
        },
        NullParser,
        NullFetcher,
    )
}

fn interactor_with_trainer_stats(
    card: Option<RaceCard>,
    trainer_surface_stats: HashMap<String, Vec<GroupStat>>,
) -> Interactor<MockRepo, NullParser, NullFetcher> {
    Interactor::new(
        MockRepo {
            card,
            odds: None,
            track_condition_stats: HashMap::new(),
            trainer_surface_stats,
            jockey_surface_stats: HashMap::new(),
        },
        NullParser,
        NullFetcher,
    )
}

fn interactor_with_jockey_stats(
    card: Option<RaceCard>,
    jockey_surface_stats: HashMap<String, Vec<GroupStat>>,
) -> Interactor<MockRepo, NullParser, NullFetcher> {
    Interactor::new(
        MockRepo {
            card,
            odds: None,
            track_condition_stats: HashMap::new(),
            trainer_surface_stats: HashMap::new(),
            jockey_surface_stats,
        },
        NullParser,
        NullFetcher,
    )
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
async fn predict_race_with_diagnostics_none_when_no_odds() {
    // オッズ未取得（find_race_odds → None）のレースは診断 None（#246-C）。
    let card = make_race_card("2026-1-tokyo-1-R1");
    let app = interactor(Some(card)); // odds: None
    let race_id = RaceId::try_from("2026-1-tokyo-1-R1").unwrap();
    let (probs, diag) = app
        .predict_race_with_diagnostics(&race_id, None, None, 5)
        .await
        .unwrap();
    assert_eq!(probs.len(), 2);
    assert!(diag.is_none(), "オッズ未取得なら診断は None");
}

#[tokio::test]
async fn predict_race_with_diagnostics_returns_axis_and_rows_with_odds() {
    // オッズありなら (軸, 各ペア行) を返す。軸は domain が決めた値で、表示側は再計算しない（#246-C）。
    let race_id_str = "2026-1-tokyo-1-R1";
    let card = make_race_card(race_id_str);
    let mut odds = paddock_domain::RaceOdds::empty(RaceId::try_from(race_id_str).unwrap());
    odds.quinella.insert(
        paddock_domain::Pair::try_from((
            HorseNum::try_from(1u32).unwrap(),
            HorseNum::try_from(2u32).unwrap(),
        ))
        .unwrap(),
        paddock_domain::OddsValue::try_from(5.0).unwrap(),
    );
    let app = interactor_with_odds(Some(card), odds);
    let race_id = RaceId::try_from(race_id_str).unwrap();
    let (_probs, diag) = app
        .predict_race_with_diagnostics(&race_id, None, None, 5)
        .await
        .unwrap();
    let diag = diag.expect("オッズありなら診断 Some");
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
        paddock_domain::OddsValue::try_from(8.0).unwrap(), // ウマA: 人気薄
    );
    odds.win.insert(
        HorseNum::try_from(2u32).unwrap(),
        paddock_domain::OddsValue::try_from(1.3).unwrap(), // ウマB: 圧倒的人気
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
