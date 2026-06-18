use paddock_domain::HorseName;

use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{HorseStatsRow, NameMatchRepository, StatsRepository};

impl<R: StatsRepository + NameMatchRepository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    pub async fn horse_stats(&self, name: &HorseName) -> Result<HorseStatsRow> {
        self.repository.horse_stats(name, None).await
    }

    /// `analyze` の部分一致候補（中間一致・名前昇順・最大 `limit` 件）。`query` は正規化済み。
    pub async fn find_horse_candidates(&self, query: &str, limit: u32) -> Result<Vec<String>> {
        self.repository
            .find_matching_horse_names(query, limit)
            .await
    }
}
