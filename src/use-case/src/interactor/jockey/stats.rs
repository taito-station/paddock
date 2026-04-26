use paddock_domain::JockeyName;

use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{JockeyStatsRow, Repository};

impl<R: Repository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    pub async fn jockey_stats(&self, name: &JockeyName) -> Result<JockeyStatsRow> {
        self.repository.jockey_stats(name).await
    }
}
