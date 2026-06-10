//! Unit tests for predict_race interactor.
//!
//! Uses in-memory mocks for all repositories to test label resolution,
//! probability normalization, and the not-found error path.

use std::collections::HashMap;

use paddock_domain::horse_result::{GateNum, HorseName, HorseNum};
use paddock_domain::{
    HorseEntry, JockeyName, Race, RaceCard, RaceId, Surface, TrackCondition, Venue,
};
use paddock_use_case::repository::{
    CourseStatsRow, FetchRecord, GroupStat, HorseStatsRow, JockeyStatsRow, Repository,
};
use paddock_use_case::{Error, Interactor, Result};

// --- helpers ----------------------------------------------------------------

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
            },
            HorseEntry {
                gate_num: GateNum::try_from(5u32).unwrap(),
                horse_num: HorseNum::try_from(2u32).unwrap(),
                horse_name: HorseName::try_from("ウマB").unwrap(),
                jockey: None,
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
}

impl Repository for MockRepo {
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
        _: &JockeyName,
        _as_of: Option<chrono::NaiveDate>,
    ) -> Result<JockeyStatsRow> {
        unimplemented!()
    }
    async fn count_races(&self) -> Result<u64> {
        Ok(0)
    }
    async fn race_exists(&self, _: &RaceId) -> Result<bool> {
        Ok(false)
    }
    async fn fetch_history_contains(&self, _: &str) -> Result<bool> {
        Ok(false)
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
        Ok(self.card.clone())
    }

    async fn find_race_odds(
        &self,
        _: &RaceId,
        _: Option<chrono::NaiveDate>,
    ) -> Result<Option<paddock_domain::RaceOdds>> {
        Ok(self.odds.clone())
    }

    async fn find_races_by_date(&self, _: chrono::NaiveDate) -> Result<Vec<Race>> {
        Ok(Vec::new())
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
    ) -> Result<Vec<(chrono::NaiveDate, paddock_domain::HorseResult)>> {
        Ok(Vec::new())
    }

    async fn find_predict_session(
        &self,
        _: chrono::NaiveDate,
    ) -> Result<Option<paddock_use_case::repository::PredictSessionRecord>> {
        Ok(None)
    }

    async fn find_predict_bets(
        &self,
        _: chrono::NaiveDate,
    ) -> Result<Vec<paddock_use_case::repository::PredictBetRecord>> {
        Ok(Vec::new())
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

fn interactor(card: Option<RaceCard>) -> Interactor<MockRepo, NullParser, NullFetcher> {
    Interactor::new(
        MockRepo {
            card,
            odds: None,
            track_condition_stats: HashMap::new(),
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

    let win_of = |probs: &[paddock_domain::HorseProbability], name: &str| {
        probs
            .iter()
            .find(|p| p.horse_name.value() == name)
            .unwrap()
            .win_prob
    };
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
