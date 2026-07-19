use chrono::Utc;
use paddock_domain::{RaceId, RaceOdds};

use crate::error::Result;
use crate::interactor::odds::OddsInteractor;
use crate::odds_scraper::OddsScraper;
use crate::repository::{OddsRepository, OddsRow, RaceOddsRecord};

impl<O: OddsScraper, R: OddsRepository> OddsInteractor<O, R> {
    /// race_id のオッズを read-through で取得する（#51, ADR 0010 / #294）。
    ///
    /// 1. `race_odds` に保存済みが complete（win + 組合せ 5 券種）なら、再スクレイプせずそれを返す。
    /// 2. complete でなければライブスクレイプし、取得した全券種(#38)を保存してフルのオッズを返す。
    ///    保存はその回の買い目には影響させない（exotic も含めて返す）。
    ///
    /// 取得できれば `Some(odds)`、未取得なら `None`。「未取得」は次の 2 つを束ねる:
    /// - スクレイプ失敗（サイト改変・開催日外・ネットワーク等）→ warn ログを出して `None`
    /// - 取得成功だが全馬券種が空（オッズ未公開）→ `None`
    ///
    /// いずれも予想フロー側ではスキップ扱いになり、1 レースの取得失敗でセッション全体を
    /// 止めない（`select_bets` を呼ばず安全に次レースへ進める設計、predict-session.md 参照）。
    pub async fn race_odds(&self, race_id: &RaceId) -> Result<Option<RaceOdds>> {
        // 1. 保存済みが complete なら再スクレイプせずに返す。
        //    cache-hit 判定は「win + 組合せ 5 券種が揃った complete スナップショット」(#294)。
        //    win あり・組合せ券種一部欠落の部分スナップショット（exotic の一過性取得失敗で生じる）は
        //    `!is_empty()` を満たすため旧判定では cache-hit して当日ずっと欠落が恒久化していた。
        //    `is_complete()` 基準にすると不完全なスナップショットは cache-miss として再スクレイプする。
        //    `race_odds` は (race_id,bet_type,combination_key) 単一行 UPSERT（save_race_odds）で、
        //    再スクレイプは欠けていた券種の行を追加するだけで既存行を消さないため、保存済みの券種集合は
        //    取得済み券種の和集合として単調に埋まり、complete に収束する（persist 側は変更不要・自己修復）。
        //    place は判定に含めない（ADR 0010 の複勝未公開時 win-only 許容を維持、is_complete 参照）。
        //    注意: JRA が一部の組合せ券種を発売しない極小頭数レースでは is_complete が常に false になり
        //    read-through を呼ぶ度に再スクレイプする。が、UPSERT で行は肥大せず、predict は 1 レース
        //    1 回・api-server は手動 refresh のみで自動ループは無いため負荷は限定的（#294 影響: 低）。
        if let Some(saved) = self.repository.find_race_odds(race_id, None).await?
            && saved.is_complete()
        {
            tracing::debug!(race_id = %race_id, "complete な保存済み race_odds を参照（再スクレイプなし）");
            return Ok(Some(saved));
        }

        // 2. complete でなければ（未保存 or 部分スナップショット）ライブスクレイプ。
        //    部分スナップショットの取り直しが #294 の中核ケース。空/失敗は従来どおりスキップ(None)。
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

    /// race_id のオッズを**必ず再スクレイプ**して新スナップショットを保存し、フルのオッズを返す（#257）。
    ///
    /// `race_odds()` の read-through はキャッシュ優先で再取得しないため、発走直前の
    /// フレッシュなオッズで EV/ROI を再計算したい監視用途には使えない。本メソッドは
    /// 保存済みの有無に関わらず常にライブスクレイプし、`persist_all` で新スナップショットを
    /// 追記する（`find_race_odds(.., None)` が最新を返すため、後続の予想はフレッシュ値を見る）。
    ///
    /// 戻り値の意味は `race_odds()` と揃える: 取得できれば `Some(odds)`、未取得（スクレイプ
    /// 失敗・全券種空＝未公開）は `None`。監視 1 レースの取得失敗で全体を止めない。
    pub async fn refresh_race_odds(&self, race_id: &RaceId) -> Result<Option<RaceOdds>> {
        match self.scraper.scrape(race_id) {
            Ok(odds) if odds.is_empty() => {
                tracing::debug!(race_id = %race_id, "オッズ再取得成功だが全馬券種が空（未公開）、スキップ");
                Ok(None)
            }
            Ok(odds) => {
                self.persist_all(race_id, &odds).await;
                Ok(Some(odds))
            }
            Err(e) => {
                tracing::warn!(race_id = %race_id, error = %e, "オッズ再取得に失敗、スキップ");
                Ok(None)
            }
        }
    }

    /// race_id の**単複のみ**を必ず再スクレイプして新スナップショットを保存する（#odds-collect）。
    ///
    /// オッズ時系列コレクタ用の軽量版 `refresh_race_odds`。組合せ 5 券種を打たず win/place
    /// （type=1・1 GET）だけを取り、`persist_all` で `race_odds`（最新・key 単位 UPSERT なので
    /// exotic 行は破壊しない）＋ `race_odds_snapshots`（append）へ保存する。全レースを終日高頻度で
    /// 貯めるため netkeiba への負荷を最小化する。戻り値の意味は `refresh_race_odds` と揃える
    /// （取得できれば `Some`、未公開/失敗は `None`・1 レースの失敗で収集ループを止めない）。
    pub async fn refresh_win_place_odds(&self, race_id: &RaceId) -> Result<Option<RaceOdds>> {
        match self.scraper.scrape_win_place(race_id) {
            Ok(odds) if odds.is_empty() => {
                tracing::debug!(race_id = %race_id, "単複再取得成功だが空（未公開）、スキップ");
                Ok(None)
            }
            Ok(odds) => {
                self.persist_all(race_id, &odds).await;
                Ok(Some(odds))
            }
            Err(e) => {
                tracing::warn!(race_id = %race_id, error = %e, "単複再取得に失敗、スキップ");
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

    use chrono::NaiveDate;
    use paddock_domain::{
        HorseNum, OddsValue, OrderedPair, OrderedTriple, Pair, PlaceOdds, RaceId, RaceOdds, Triple,
    };

    use crate::error::{Error, Result};
    use crate::interactor::odds::OddsInteractor;
    use crate::odds_scraper::OddsScraper;
    use crate::repository::{OddsRepository, RaceOddsRecord};

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

    impl OddsRepository for FakeRepo {
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
        async fn find_race_odds_morning(
            &self,
            _race_id: &RaceId,
        ) -> Result<Option<crate::repository::MorningRaceOdds>> {
            Ok(None)
        }
        async fn purge_race_odds_snapshots(&self, _before: NaiveDate) -> Result<u64> {
            Ok(0)
        }
        async fn count_race_odds_snapshots_before(&self, _before: NaiveDate) -> Result<u64> {
            Ok(0)
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
        // 保存済みが complete（win + 組合せ 5 券種）なら scrape を呼ばずにそれを返す（#294）。
        let scraper = FakeScraper::new(|_| panic!("scrape は呼ばれてはならない"));
        let repo = FakeRepo {
            preset: Some(odds_all_types(race_id())),
            ..Default::default()
        };
        let interactor = OddsInteractor::new(scraper, repo);

        let got = interactor.race_odds(&race_id()).await.unwrap();
        assert!(got.is_some_and(|o| o.is_complete()));
        assert_eq!(*interactor.scraper.calls.lock().unwrap(), 0);
        assert!(interactor.repository.saved.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn rescrapes_when_saved_snapshot_incomplete() {
        // #294: 保存済みが部分スナップショット（win+place のみ・組合せ券種欠落）の場合は
        // cache-miss として再スクレイプし、complete を取り直して persist する。
        // exotic の一過性取得失敗で生じた欠落が当日恒久化するのを防ぐ。
        let scraper = FakeScraper::new(|rid| Ok(odds_all_types(rid.clone())));
        let repo = FakeRepo {
            preset: Some(odds_win_place(race_id())), // 組合せ券種が無い＝is_complete()=false
            ..Default::default()
        };
        let interactor = OddsInteractor::new(scraper, repo);

        let got = interactor.race_odds(&race_id()).await.unwrap();
        assert!(
            got.is_some_and(|o| o.is_complete()),
            "再スクレイプで complete を返す"
        );
        assert_eq!(
            *interactor.scraper.calls.lock().unwrap(),
            1,
            "部分スナップショットは cache-miss として再スクレイプする"
        );
        assert_eq!(
            interactor.repository.saved.lock().unwrap().len(),
            1,
            "再取得した complete スナップショットを追記保存する"
        );
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

    #[tokio::test]
    async fn refresh_scrapes_and_persists_even_when_saved() {
        // #257: refresh は read-through と違い、保存済みがあっても必ず再スクレイプし
        // 新スナップショットを保存する（発走直前のフレッシュなオッズを得るため）。
        let scraper = FakeScraper::new(|rid| Ok(odds_all_types(rid.clone())));
        let repo = FakeRepo {
            preset: Some(odds_with_win(race_id())),
            ..Default::default()
        };
        let interactor = OddsInteractor::new(scraper, repo);

        let got = interactor.refresh_race_odds(&race_id()).await.unwrap();
        assert!(got.is_some_and(|o| !o.is_empty()));
        // 保存済みがあっても scrape は必ず 1 回呼ばれる（read-through との決定的な違い）。
        assert_eq!(*interactor.scraper.calls.lock().unwrap(), 1);
        // 新スナップショットが 1 レコード追記される。
        assert_eq!(interactor.repository.saved.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn refresh_win_place_persists_only_win_place() {
        // #odds-collect: 単複限定 refresh は win/place だけを保存する（組合せ券種は打たない）。
        // FakeScraper は scrape() で全券種を返すが、trait 既定の scrape_win_place が win/place に絞る。
        let scraper = FakeScraper::new(|rid| Ok(odds_all_types(rid.clone())));
        let interactor = OddsInteractor::new(scraper, FakeRepo::default());

        let got = interactor.refresh_win_place_odds(&race_id()).await.unwrap();
        assert!(got.is_some_and(|o| !o.is_empty()));
        assert_eq!(*interactor.scraper.calls.lock().unwrap(), 1);

        let saved = interactor.repository.saved.lock().unwrap();
        assert_eq!(saved.len(), 1, "新スナップショットを 1 レコード追記");
        let rows = &saved[0].rows;
        let count = |bt: &str| rows.iter().filter(|r| r.bet_type == bt).count();
        assert_eq!(count("win"), 1, "win を保存");
        assert_eq!(count("place"), 1, "place を保存");
        for bt in ["quinella", "wide", "exacta", "trio", "trifecta"] {
            assert_eq!(count(bt), 0, "{bt} は単複限定なので保存しない");
        }
    }

    #[tokio::test]
    async fn refresh_win_place_returns_none_when_empty_or_error() {
        // 未公開（空）も失敗も None・保存なし（refresh_race_odds と挙動を揃える）。
        let empty = OddsInteractor::new(
            FakeScraper::new(|rid| Ok(RaceOdds::empty(rid.clone()))),
            FakeRepo::default(),
        );
        assert!(
            empty
                .refresh_win_place_odds(&race_id())
                .await
                .unwrap()
                .is_none()
        );
        assert!(empty.repository.saved.lock().unwrap().is_empty());

        let errored = OddsInteractor::new(
            FakeScraper::new(|_| Err(Error::Internal("nav failed".into()))),
            FakeRepo::default(),
        );
        assert!(
            errored
                .refresh_win_place_odds(&race_id())
                .await
                .unwrap()
                .is_none()
        );
        assert!(errored.repository.saved.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn refresh_returns_none_when_empty_or_error() {
        // 未公開（全券種空）も失敗も None。保存はしない（race_odds() と挙動を揃える）。
        let empty = OddsInteractor::new(
            FakeScraper::new(|rid| Ok(RaceOdds::empty(rid.clone()))),
            FakeRepo::default(),
        );
        assert!(empty.refresh_race_odds(&race_id()).await.unwrap().is_none());
        assert!(empty.repository.saved.lock().unwrap().is_empty());

        let errored = OddsInteractor::new(
            FakeScraper::new(|_| Err(Error::Internal("nav failed".into()))),
            FakeRepo::default(),
        );
        assert!(
            errored
                .refresh_race_odds(&race_id())
                .await
                .unwrap()
                .is_none()
        );
        assert!(errored.repository.saved.lock().unwrap().is_empty());
    }
}
