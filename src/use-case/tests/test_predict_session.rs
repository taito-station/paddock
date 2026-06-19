//! create_predict_session / record_race_outcome の不変条件を in-memory モックで固定する単体テスト。
//!
//! #164 で CLI predict のセッション作成・残高ガード・残高/累計計算を use-case 層へ一本化した結果、
//! これらのメソッドが不変条件の唯一の強制点になった。Postgres を要する統合テスト（#160）とは別に、
//! ガード（budget>0・二重作成 Conflict・Σstake≤balance・二重記録 Conflict・未作成 NotFound）と
//! 残高/累計更新を DB 非依存で検証する。

use std::sync::Mutex;

use chrono::{DateTime, NaiveDate, Utc};
use paddock_domain::RaceId;
use paddock_use_case::repository::{
    PredictBetRecord, PredictRaceConditionRecord, PredictSessionRecord, PredictSessionRepository,
};
use paddock_use_case::{Error, Interactor, Result};

fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 6, 14).unwrap()
}

fn race_id(s: &str) -> RaceId {
    RaceId::try_from(s).unwrap()
}

fn bet(race: &str, stake: u64, payout: u64) -> PredictBetRecord {
    PredictBetRecord {
        race_id: race_id(race),
        bet_type: "win".to_string(),
        combination: "8".to_string(),
        stake,
        payout,
        ev: 1.5,
    }
}

// --- in-memory モック Repository -------------------------------------------

/// セッションヘッダと買い目を保持し、create→record の読み書きを再現する最小モック。
/// セッション系以外（settle / 馬場入力 / id 付き取得）はこのテストでは未使用。
#[derive(Default)]
struct MockRepo {
    session: Mutex<Option<PredictSessionRecord>>,
    bets: Mutex<Vec<PredictBetRecord>>,
}

impl PredictSessionRepository for MockRepo {
    async fn find_predict_session(&self, _: NaiveDate) -> Result<Option<PredictSessionRecord>> {
        Ok(self.session.lock().unwrap().clone())
    }

    async fn find_predict_bets(&self, _: NaiveDate) -> Result<Vec<PredictBetRecord>> {
        Ok(self.bets.lock().unwrap().clone())
    }

    async fn save_predict_session(&self, session: &PredictSessionRecord) -> Result<()> {
        *self.session.lock().unwrap() = Some(session.clone());
        Ok(())
    }

    async fn save_race_outcome(
        &self,
        session: &PredictSessionRecord,
        _: &RaceId,
        bets: &[PredictBetRecord],
    ) -> Result<()> {
        *self.session.lock().unwrap() = Some(session.clone());
        self.bets.lock().unwrap().extend_from_slice(bets);
        Ok(())
    }

    // --- 以降はこのテストでは未使用 ---
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

struct NullParser;
impl paddock_use_case::PdfParser for NullParser {
    fn parse(&self, _: &[u8]) -> Result<Vec<paddock_domain::Race>> {
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

fn interactor() -> Interactor<MockRepo, NullParser, NullFetcher> {
    Interactor::new(MockRepo::default(), NullParser, NullFetcher)
}

// --- create_predict_session -------------------------------------------------

#[tokio::test]
async fn create_initializes_balance_and_persists() {
    let app = interactor();

    let session = app.create_predict_session(date(), 10000).await.unwrap();

    assert_eq!(session.budget, 10000);
    assert_eq!(session.balance, 10000);
    assert_eq!(session.total_bet, 0);
    assert_eq!(session.total_payout, 0);
    assert!(!session.completed);
    // 開始時点でヘッダが保存され、find で引ける。
    let stored = app.find_predict_session(date()).await.unwrap().unwrap();
    assert_eq!(stored.balance, 10000);
}

#[tokio::test]
async fn create_rejects_zero_budget() {
    let app = interactor();

    let err = app.create_predict_session(date(), 0).await.unwrap_err();
    assert!(matches!(err, Error::InvalidArgument(_)));
    // 拒否時はヘッダを保存しない。
    assert!(app.find_predict_session(date()).await.unwrap().is_none());
}

#[tokio::test]
async fn create_rejects_duplicate() {
    let app = interactor();
    app.create_predict_session(date(), 10000).await.unwrap();

    let err = app.create_predict_session(date(), 5000).await.unwrap_err();
    assert!(matches!(err, Error::Conflict(_)));
}

// --- record_race_outcome ----------------------------------------------------

#[tokio::test]
async fn record_updates_balance_and_totals() {
    let app = interactor();
    app.create_predict_session(date(), 10000).await.unwrap();

    let updated = app
        .record_race_outcome(
            date(),
            &race_id("2026-4-tokyo-1-1R"),
            vec![bet("2026-4-tokyo-1-1R", 1000, 2500)],
        )
        .await
        .unwrap();

    // balance = 10000 - 1000 + 2500、累計も加算される。
    assert_eq!(updated.balance, 11500);
    assert_eq!(updated.total_bet, 1000);
    assert_eq!(updated.total_payout, 2500);
    // 永続化され、find で引ける。
    let stored = app.find_predict_session(date()).await.unwrap().unwrap();
    assert_eq!(stored.balance, 11500);
}

#[tokio::test]
async fn record_rejects_overstake_and_leaves_session_unchanged() {
    let app = interactor();
    app.create_predict_session(date(), 10000).await.unwrap();

    let err = app
        .record_race_outcome(
            date(),
            &race_id("2026-4-tokyo-1-1R"),
            vec![bet("2026-4-tokyo-1-1R", 15000, 0)],
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::InvalidArgument(_)));
    // 残高ガードで弾かれた後、セッションは不変（balance=budget のまま）。
    let stored = app.find_predict_session(date()).await.unwrap().unwrap();
    assert_eq!(stored.balance, 10000);
    assert_eq!(stored.total_bet, 0);
}

#[tokio::test]
async fn record_rejects_duplicate_race() {
    let app = interactor();
    app.create_predict_session(date(), 10000).await.unwrap();
    app.record_race_outcome(
        date(),
        &race_id("2026-4-tokyo-1-1R"),
        vec![bet("2026-4-tokyo-1-1R", 1000, 0)],
    )
    .await
    .unwrap();

    // 同一レースへ買い目ありで再記録すると Conflict（買い目重複＋残高二重適用を防ぐ）。
    let err = app
        .record_race_outcome(
            date(),
            &race_id("2026-4-tokyo-1-1R"),
            vec![bet("2026-4-tokyo-1-1R", 500, 0)],
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)));
}

#[tokio::test]
async fn record_without_session_is_not_found() {
    let app = interactor();

    let err = app
        .record_race_outcome(
            date(),
            &race_id("2026-4-tokyo-1-1R"),
            vec![bet("2026-4-tokyo-1-1R", 1000, 0)],
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)));
}
