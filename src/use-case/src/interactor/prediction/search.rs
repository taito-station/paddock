use crate::error::Result;
use crate::interactor::Interactor;
use crate::repository::{PadPredictionRepository, PredictionFilter, PredictionSearchResult};

impl<R: PadPredictionRepository> Interactor<R> {
    /// 予想を横断検索する（#145）。フィルタの妥当性（範囲の逆転など）は呼び出し側で検証済みとし、
    /// ここでは repository へ委譲する。
    pub async fn search_predictions(
        &self,
        filter: PredictionFilter,
    ) -> Result<PredictionSearchResult> {
        self.repository.search_predictions(&filter).await
    }
}
