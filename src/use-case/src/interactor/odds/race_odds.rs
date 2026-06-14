use chrono::Utc;
use paddock_domain::{RaceId, RaceOdds};

use crate::error::Result;
use crate::interactor::odds::OddsInteractor;
use crate::odds_scraper::OddsScraper;
use crate::repository::{OddsRow, RaceOddsRecord, Repository};

impl<O: OddsScraper, R: Repository> OddsInteractor<O, R> {
    /// race_id のオッズを read-through で取得する（#51, ADR 0010）。
    ///
    /// 1. `race_odds` に保存済み（単勝・複勝）があれば、再スクレイプせずそれを返す。
    /// 2. 無ければライブスクレイプし、取得できた単勝・複勝を保存してからフルのオッズを返す。
    ///    保存はその回の買い目には影響させない（exotic も含めて返す）。
    ///
    /// 取得できれば `Some(odds)`、未取得なら `None`。「未取得」は次の 2 つを束ねる:
    /// - スクレイプ失敗（サイト改変・開催日外・ネットワーク等）→ warn ログを出して `None`
    /// - 取得成功だが全馬券種が空（オッズ未公開）→ `None`
    ///
    /// いずれも予想フロー側ではスキップ扱いになり、1 レースの取得失敗でセッション全体を
    /// 止めない（`select_bets` を呼ばず安全に次レースへ進める設計、predict-session.md 参照）。
    pub async fn race_odds(&self, race_id: &RaceId) -> Result<Option<RaceOdds>> {
        // 1. 保存済みがあれば再スクレイプせずに返す。
        //    cache-hit 判定は「いずれかの馬券種が保存済み」(= `!is_empty()`)。#38 で組合せ券種も
        //    保存・読み戻すようになったため、cache-hit 時も exotic を含むフルのオッズを返す。
        //    find_race_odds は行が無ければ None を返すため `!is_empty()` は実質二重ガード（防御目的）。
        if let Some(saved) = self.repository.find_race_odds(race_id, None).await?
            && !saved.is_empty()
        {
            tracing::debug!(race_id = %race_id, "保存済み race_odds を参照（再スクレイプなし）");
            return Ok(Some(saved));
        }

        // 2. 未保存ならライブスクレイプ。空/失敗は従来どおりスキップ(None)。
        match self.scraper.scrape(race_id) {
            Ok(odds) if odds.is_empty() => {
                // 取得は成功したが全馬券種が空（未公開）。スクレイプ失敗（warn）と
                // 区別できるよう debug で記録し、運用時に原因を切り分けられるようにする。
                tracing::debug!(race_id = %race_id, "オッズ取得成功だが全馬券種が空（未公開）、スキップ");
                Ok(None)
            }
            Ok(odds) => {
                // 取得できた全券種を永続化（#38）。保存失敗は予想を止めず warn のみ。
                self.persist_all(race_id, &odds).await;
                // フルのオッズはその回の買い目にそのまま使う。
                Ok(Some(odds))
            }
            Err(e) => {
                tracing::warn!(race_id = %race_id, error = %e, "オッズ取得に失敗、スキップ");
                Ok(None)
            }
        }
    }

    /// スクレイプで得た全券種のオッズを `race_odds` に保存する。複勝・ワイドは幅 odds
    /// （下限=odds, 上限=odds_high）。スクレイプ由来は人気を持たないため popularity は None。
    /// 保存失敗は予想フローを止めず warn ログのみ（次回参照時に取り直せる）。
    async fn persist_all(&self, race_id: &RaceId, odds: &RaceOdds) {
        let capacity = odds.win.len()
            + odds.place.len()
            + odds.quinella.len()
            + odds.wide.len()
            + odds.exacta.len()
            + odds.trio.len()
            + odds.trifecta.len();
        let mut rows: Vec<OddsRow> = Vec::with_capacity(capacity);
        for (horse, ov) in &odds.win {
            rows.push(OddsRow::win(horse.value(), ov.value(), None));
        }
        for (horse, place) in &odds.place {
            rows.push(OddsRow::place(
                horse.value(),
                place.low.value(),
                place.high.value(),
                None,
            ));
        }
        for (pair, ov) in &odds.quinella {
            rows.push(OddsRow::quinella(*pair, ov.value()));
        }
        for (pair, band) in &odds.wide {
            rows.push(OddsRow::wide(*pair, band.low.value(), band.high.value()));
        }
        for (pair, ov) in &odds.exacta {
            rows.push(OddsRow::exacta(*pair, ov.value()));
        }
        for (triple, ov) in &odds.trio {
            rows.push(OddsRow::trio(*triple, ov.value()));
        }
        for (triple, ov) in &odds.trifecta {
            rows.push(OddsRow::trifecta(*triple, ov.value()));
        }
        if rows.is_empty() {
            return;
        }
        let record = RaceOddsRecord {
            race_id: race_id.clone(),
            fetched_at: Utc::now(),
            rows,
        };
        if let Err(e) = self.repository.save_race_odds(&record).await {
            tracing::warn!(race_id = %race_id, error = %e, "race_odds の保存に失敗（予想は継続）");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use chrono::{DateTime, NaiveDate, Utc};
    use paddock_domain::{
        HorseName, HorseNum, HorseResult, JockeyName, OddsValue, OrderedPair, OrderedTriple, Pair,
        PlaceOdds, Race, RaceCard, RaceId, RaceOdds, Surface, TrainerName, Triple, Venue,
    };

    use crate::error::{Error, Result};
    use crate::interactor::odds::OddsInteractor;
    use crate::odds_scraper::OddsScraper;
    use crate::repository::{
        CourseStatsRow, FetchRecord, HorseStatsRow, JockeyStatsRow, PredictBetRecord,
        PredictRaceConditionRecord, PredictSessionRecord, RaceOddsRecord, Repository,
        TrainerStatsRow,
    };

    /// テスト用の OddsScraper。scrape の戻り値を差し替えつつ呼び出し回数を数える。
    struct FakeScraper {
        result: fn(&RaceId) -> Result<RaceOdds>,
        calls: Mutex<usize>,
    }

    impl FakeScraper {
        fn new(result: fn(&RaceId) -> Result<RaceOdds>) -> Self {
            Self {
                result,
                calls: Mutex::new(0),
            }
        }
    }

    impl OddsScraper for FakeScraper {
        fn scrape(&self, race_id: &RaceId) -> Result<RaceOdds> {
            *self.calls.lock().unwrap() += 1;
            (self.result)(race_id)
        }
    }

    /// 保存済みオッズの有無と save 呼び出しを記録するだけの Repository フェイク。
    #[derive(Default)]
    struct FakeRepo {
        preset: Option<RaceOdds>,
        saved: Mutex<Vec<RaceOddsRecord>>,
    }

    impl Repository for FakeRepo {
        async fn find_race_odds(
            &self,
            _race_id: &RaceId,
            _as_of: Option<NaiveDate>,
        ) -> Result<Option<RaceOdds>> {
            Ok(self.preset.clone())
        }
        async fn save_race_odds(&self, record: &RaceOddsRecord) -> Result<()> {
            self.saved.lock().unwrap().push(record.clone());
            Ok(())
        }
        // --- 以降は本テストで未使用 ---
        async fn save_race(&self, _: &Race) -> Result<()> {
            unimplemented!()
        }
        async fn upsert_horse_history(
            &self,
            _: &paddock_domain::HorseId,
            _: &[crate::HorsePastRun],
        ) -> Result<usize> {
            unimplemented!()
        }
        async fn backfill_results_horse_ids(&self) -> Result<u64> {
            unimplemented!()
        }
        async fn find_matching_horse_names(&self, _: &str, _: u32) -> Result<Vec<String>> {
            unimplemented!()
        }
        async fn find_matching_jockey_names(&self, _: &str, _: u32) -> Result<Vec<String>> {
            unimplemented!()
        }
        async fn find_matching_trainer_names(&self, _: &str, _: u32) -> Result<Vec<String>> {
            unimplemented!()
        }
        async fn horse_stats(&self, _: &HorseName, _: Option<NaiveDate>) -> Result<HorseStatsRow> {
            unimplemented!()
        }
        async fn course_stats(
            &self,
            _: Venue,
            _: u32,
            _: Surface,
            _: Option<NaiveDate>,
        ) -> Result<CourseStatsRow> {
            unimplemented!()
        }
        async fn jockey_stats(
            &self,
            _: &JockeyName,
            _: Option<NaiveDate>,
        ) -> Result<JockeyStatsRow> {
            unimplemented!()
        }
        async fn trainer_stats(
            &self,
            _: &TrainerName,
            _: Option<NaiveDate>,
        ) -> Result<TrainerStatsRow> {
            unimplemented!()
        }
        async fn find_finished_races_between(
            &self,
            _: NaiveDate,
            _: NaiveDate,
        ) -> Result<Vec<Race>> {
            unimplemented!()
        }
        async fn find_recent_runs(
            &self,
            _: &HorseName,
            _: NaiveDate,
            _: u32,
        ) -> Result<Vec<(NaiveDate, HorseResult)>> {
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
        async fn find_predict_session(&self, _: NaiveDate) -> Result<Option<PredictSessionRecord>> {
            unimplemented!()
        }
        async fn find_predict_bets(&self, _: NaiveDate) -> Result<Vec<PredictBetRecord>> {
            unimplemented!()
        }
        async fn find_predict_bets_with_id(
            &self,
            _: NaiveDate,
        ) -> Result<Vec<(i64, PredictBetRecord)>> {
            unimplemented!()
        }
        async fn settle_predict_session(
            &self,
            _: &PredictSessionRecord,
            _: &[(i64, u64)],
        ) -> Result<()> {
            unimplemented!()
        }
        async fn save_predict_session(&self, _: &PredictSessionRecord) -> Result<()> {
            unimplemented!()
        }
        async fn save_race_outcome(
            &self,
            _: &PredictSessionRecord,
            _: &RaceId,
            _: &[PredictBetRecord],
        ) -> Result<()> {
            unimplemented!()
        }
        async fn find_predict_race_conditions(
            &self,
            _: NaiveDate,
        ) -> Result<Vec<PredictRaceConditionRecord>> {
            unimplemented!()
        }
        async fn save_predict_race_condition(
            &self,
            _: NaiveDate,
            _: &PredictRaceConditionRecord,
            _: DateTime<Utc>,
        ) -> Result<()> {
            unimplemented!()
        }
    }

    fn race_id() -> RaceId {
        RaceId::try_from("2026-3-nakayama-8-1R").unwrap()
    }

    fn odds_with_win(race_id: RaceId) -> RaceOdds {
        let mut odds = RaceOdds::empty(race_id);
        odds.win.insert(
            HorseNum::try_from(1).unwrap(),
            OddsValue::try_from(3.5).unwrap(),
        );
        odds
    }

    fn odds_win_place(race_id: RaceId) -> RaceOdds {
        let mut odds = odds_with_win(race_id);
        odds.place.insert(
            HorseNum::try_from(1).unwrap(),
            PlaceOdds::try_from((
                OddsValue::try_from(1.5).unwrap(),
                OddsValue::try_from(2.0).unwrap(),
            ))
            .unwrap(),
        );
        odds
    }

    #[tokio::test]
    async fn returns_saved_without_scraping() {
        // 保存済みがあれば scrape を呼ばずにそれを返す。
        let scraper = FakeScraper::new(|_| panic!("scrape は呼ばれてはならない"));
        let repo = FakeRepo {
            preset: Some(odds_with_win(race_id())),
            ..Default::default()
        };
        let interactor = OddsInteractor::new(scraper, repo);

        let got = interactor.race_odds(&race_id()).await.unwrap();
        assert!(got.is_some_and(|o| !o.is_empty()));
        assert_eq!(*interactor.scraper.calls.lock().unwrap(), 0);
        assert!(interactor.repository.saved.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn scrapes_and_persists_when_not_saved() {
        // 未保存ならスクレイプし、単勝・複勝を保存してフルのオッズを返す。
        let scraper = FakeScraper::new(|rid| Ok(odds_win_place(rid.clone())));
        let interactor = OddsInteractor::new(scraper, FakeRepo::default());

        let got = interactor.race_odds(&race_id()).await.unwrap();
        assert!(got.is_some_and(|o| !o.is_empty()));
        assert_eq!(*interactor.scraper.calls.lock().unwrap(), 1);

        let saved = interactor.repository.saved.lock().unwrap();
        assert_eq!(saved.len(), 1, "単勝・複勝を 1 レコードで保存");
        let rows = &saved[0].rows;
        assert_eq!(rows.iter().filter(|r| r.bet_type == "win").count(), 1);
        let place: Vec<_> = rows.iter().filter(|r| r.bet_type == "place").collect();
        assert_eq!(place.len(), 1);
        assert!((place[0].odds - 1.5).abs() < 1e-9);
        assert_eq!(place[0].odds_high, Some(2.0));
    }

    fn odds_all_types(race_id: RaceId) -> RaceOdds {
        let mut odds = odds_win_place(race_id);
        let h = |n: u32| HorseNum::try_from(n).unwrap();
        let ov = |v: f64| OddsValue::try_from(v).unwrap();
        odds.quinella
            .insert(Pair::try_from((h(1), h(2))).unwrap(), ov(12.4));
        odds.wide.insert(
            Pair::try_from((h(1), h(2))).unwrap(),
            PlaceOdds::try_from((ov(3.1), ov(4.8))).unwrap(),
        );
        odds.exacta
            .insert(OrderedPair::try_from((h(2), h(1))).unwrap(), ov(25.0));
        odds.trio
            .insert(Triple::try_from((h(1), h(2), h(3))).unwrap(), ov(88.0));
        odds.trifecta.insert(
            OrderedTriple::try_from((h(3), h(1), h(2))).unwrap(),
            ov(410.0),
        );
        odds
    }

    #[tokio::test]
    async fn persists_all_bet_types_when_scraped() {
        // #38: スクレイプで得た組合せ券種も含め全券種を保存する。
        let scraper = FakeScraper::new(|rid| Ok(odds_all_types(rid.clone())));
        let interactor = OddsInteractor::new(scraper, FakeRepo::default());

        interactor.race_odds(&race_id()).await.unwrap();

        let saved = interactor.repository.saved.lock().unwrap();
        let rows = &saved[0].rows;
        let count = |bt: &str| rows.iter().filter(|r| r.bet_type == bt).count();
        for bt in [
            "win", "place", "quinella", "wide", "exacta", "trio", "trifecta",
        ] {
            assert_eq!(count(bt), 1, "{bt} が 1 行保存されること");
        }
        // ワイドは複勝同様に幅 odds（odds_high 付き）で保存される。
        let wide = rows.iter().find(|r| r.bet_type == "wide").unwrap();
        assert_eq!(wide.combination_key, "1-2");
        assert_eq!(wide.odds_high, Some(4.8));
        // 馬単はキーの順序を保持（2>1）。
        let exacta = rows.iter().find(|r| r.bet_type == "exacta").unwrap();
        assert_eq!(exacta.combination_key, "2>1");
    }

    #[tokio::test]
    async fn returns_none_when_odds_empty() {
        // 取得成功だが未公開（全馬券種が空）→ スキップ扱いの None。保存もしない。
        let scraper = FakeScraper::new(|rid| Ok(RaceOdds::empty(rid.clone())));
        let interactor = OddsInteractor::new(scraper, FakeRepo::default());

        let got = interactor.race_odds(&race_id()).await.unwrap();
        assert!(got.is_none());
        assert!(interactor.repository.saved.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn returns_none_on_scrape_error() {
        // スクレイプ失敗はセッションを止めず None で安全にスキップ。
        let scraper = FakeScraper::new(|_| Err(Error::Internal("navigation failed".into())));
        let interactor = OddsInteractor::new(scraper, FakeRepo::default());

        let got = interactor.race_odds(&race_id()).await.unwrap();
        assert!(got.is_none());
        assert!(interactor.repository.saved.lock().unwrap().is_empty());
    }
}
