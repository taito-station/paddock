use paddock_domain::PadPrediction;

use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::PadPredictionRepository;

impl<R: PadPredictionRepository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// 予想 1 件を主キーで取得する（#145・個別予想ビュー）。未存在なら `None`。
    pub async fn prediction_detail(&self, prediction_id: i64) -> Result<Option<PadPrediction>> {
        self.repository
            .find_pad_prediction_by_id(prediction_id)
            .await
    }
}
