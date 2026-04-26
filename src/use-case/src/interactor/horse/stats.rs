use paddock_domain::HorseName;

use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{HorseStatsRow, Repository};

impl<R: Repository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    pub async fn horse_stats(&self, name: &HorseName) -> Result<HorseStatsRow> {
        self.repository.horse_stats(name).await
    }
}
