use chrono::{NaiveDate, Utc};
use paddock_domain::{RaceId, TrackCondition};

use crate::error::{Error, Result};
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{
    PredictBetRecord, PredictRaceConditionRecord, PredictSessionRecord, Repository,
};

impl<R: Repository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// 指定日の予想セッションを取得する（未作成なら `None`）。
    pub async fn find_predict_session(
        &self,
        date: NaiveDate,
    ) -> Result<Option<PredictSessionRecord>> {
        self.repository.find_predict_session(date).await
    }

    /// 指定日のセッションで購入済みの買い目を取得する。
    pub async fn find_predict_bets(&self, date: NaiveDate) -> Result<Vec<PredictBetRecord>> {
        self.repository.find_predict_bets(date).await
    }

    /// 予想セッションを新規作成する（REST API #53）。不変条件を use-case で強制し API/CLI で共有する:
    /// budget は 1 以上（0 は `InvalidArgument`）、同一開催日の二重作成は `Conflict`。
    /// `balance = budget`・累計 0・未完了で保存し、作成したレコードを返す。時刻はこの層で注入する。
    pub async fn create_predict_session(
        &self,
        date: NaiveDate,
        budget: u64,
    ) -> Result<PredictSessionRecord> {
        if budget == 0 {
            return Err(Error::InvalidArgument(
                "budget must be greater than 0".into(),
            ));
        }
        if self.repository.find_predict_session(date).await?.is_some() {
            return Err(Error::Conflict(format!(
                "session for {date} already exists"
            )));
        }
        let now = Utc::now();
        let session = PredictSessionRecord {
            date,
            budget,
            balance: budget,
            total_bet: 0,
            total_payout: 0,
            completed: false,
            created_at: now,
            updated_at: now,
        };
        self.repository.save_predict_session(&session).await?;
        Ok(session)
    }

    /// 1 レース分の買い目・払戻を記録し残高/累計を更新する（REST API #53）。残高ガード
    /// （`Σstake ≤ balance`、超過は `InvalidArgument` で状態不変）を強制し、セッションヘッダ更新と
    /// 当該レースの買い目追記を 1 トランザクション（`save_race_outcome`）で保存する。更新後レコードを返す。
    /// セッション未作成は `NotFound`。`balance` は u64 で常に 0 以上に保たれる（ガードにより underflow しない）。
    pub async fn record_race_outcome(
        &self,
        date: NaiveDate,
        race_id: &RaceId,
        bets: Vec<PredictBetRecord>,
    ) -> Result<PredictSessionRecord> {
        let mut session = self
            .repository
            .find_predict_session(date)
            .await?
            .ok_or_else(|| Error::NotFound(format!("session for {date} not found")))?;

        let total_stake: u64 = bets.iter().map(|b| b.stake).sum();
        if total_stake > session.balance {
            return Err(Error::InvalidArgument(format!(
                "total stake {total_stake} exceeds balance {}",
                session.balance
            )));
        }
        let total_payout: u64 = bets.iter().map(|b| b.payout).sum();

        session.balance = session.balance - total_stake + total_payout;
        session.total_bet += total_stake;
        session.total_payout += total_payout;
        session.updated_at = Utc::now();

        self.repository
            .save_race_outcome(&session, race_id, &bets)
            .await?;
        Ok(session)
    }

    /// 指定日のセッション収支と買い目明細をまとめて返す（REST API #53 のサマリ）。未作成は `NotFound`。
    pub async fn session_summary(
        &self,
        date: NaiveDate,
    ) -> Result<(PredictSessionRecord, Vec<PredictBetRecord>)> {
        let session = self
            .repository
            .find_predict_session(date)
            .await?
            .ok_or_else(|| Error::NotFound(format!("session for {date} not found")))?;
        let bets = self.repository.find_predict_bets(date).await?;
        Ok((session, bets))
    }

    /// 予想セッションのヘッダを upsert する（新規作成・完了マーク用）。
    pub async fn save_predict_session(&self, session: &PredictSessionRecord) -> Result<()> {
        self.repository.save_predict_session(session).await
    }

    /// 1 レース分の確定結果（セッション更新＋買い目）を 1 トランザクションで保存する。
    pub async fn save_race_outcome(
        &self,
        session: &PredictSessionRecord,
        race_id: &RaceId,
        bets: &[PredictBetRecord],
    ) -> Result<()> {
        self.repository
            .save_race_outcome(session, race_id, bets)
            .await
    }

    /// 指定日のセッションで記録済みの馬場入力を取得する（`--resume` のデフォルト提示用）。
    pub async fn find_predict_race_conditions(
        &self,
        date: NaiveDate,
    ) -> Result<Vec<PredictRaceConditionRecord>> {
        self.repository.find_predict_race_conditions(date).await
    }

    /// 1 レース分の馬場入力を記録する。記録時刻 `Utc::now()` はこの use-case 層で注入し、
    /// gateway を時計から独立に保つ（時刻注入の境界は本メソッド）。
    pub async fn save_predict_race_condition(
        &self,
        date: NaiveDate,
        race_id: &RaceId,
        track_condition: Option<TrackCondition>,
    ) -> Result<()> {
        let record = PredictRaceConditionRecord {
            race_id: race_id.clone(),
            track_condition,
        };
        self.repository
            .save_predict_race_condition(date, &record, Utc::now())
            .await
    }
}
