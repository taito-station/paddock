use paddock_domain::{RaceId, RaceOdds};

use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::Repository;

impl<R: Repository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// race_id のオッズを取得する。未取得の場合は `None`。
    pub async fn race_odds(&self, race_id: &RaceId) -> Result<Option<RaceOdds>> {
        self.repository.find_race_odds(race_id).await
    }
}
