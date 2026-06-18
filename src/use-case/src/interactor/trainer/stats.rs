use paddock_domain::TrainerName;

use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{NameMatchRepository, StatsRepository, TrainerStatsRow};

impl<R: StatsRepository + NameMatchRepository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    pub async fn trainer_stats(&self, name: &TrainerName) -> Result<TrainerStatsRow> {
        self.repository.trainer_stats(name, None).await
    }

    /// `analyze` の部分一致候補（中間一致・名前昇順・最大 `limit` 件）。`query` は正規化済み。
    pub async fn find_trainer_candidates(&self, query: &str, limit: u32) -> Result<Vec<String>> {
        self.repository
            .find_matching_trainer_names(query, limit)
            .await
    }
}
