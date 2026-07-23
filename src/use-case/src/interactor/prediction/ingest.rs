use chrono::{DateTime, Utc};
use paddock_domain::PadPrediction;

use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::PadPredictionRepository;

impl<R: PadPredictionRepository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// 予想 1 件（印・短評・買い目・結果）を取り込む（#456）。`(date, venue, race_num)` で
    /// upsert する repository へ委譲するだけの薄いユースケース。`now` は app 層が注入し
    /// gateway を時計から独立に保つ（`save_pad_prediction` の契約と同じ）。
    pub async fn ingest_pad_prediction(
        &self,
        prediction: &PadPrediction,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.repository.save_pad_prediction(prediction, now).await
    }
}
