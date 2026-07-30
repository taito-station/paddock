use crate::error::Result;
use crate::interactor::Interactor;
use crate::repository::{MarkStatRow, MarkStatsFilter, PadPredictionRepository};

impl<R: PadPredictionRepository> Interactor<R> {
    /// 印別の的中率集計を返す（#145・集計の入口）。母集団は結果記録済みの予想。
    pub async fn prediction_mark_stats(&self, filter: MarkStatsFilter) -> Result<Vec<MarkStatRow>> {
        self.repository.prediction_mark_stats(&filter).await
    }
}
