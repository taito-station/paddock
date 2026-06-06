use chrono::NaiveDate;
use paddock_domain::RaceId;

use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{PredictBetRecord, PredictSessionRecord, Repository};

impl<R: Repository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// 指定日の予想セッションを取得する（未作成なら `None`）。
    pub async fn find_predict_session(
        &self,
        date: NaiveDate,
    ) -> Result<Option<PredictSessionRecord>> {
        self.repository.find_predict_session(date).await
    }

    /// 指定日のセッションで購入済みの買い目を取得する。
    pub async fn predict_bets(&self, date: NaiveDate) -> Result<Vec<PredictBetRecord>> {
        self.repository.find_predict_bets(date).await
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
}
