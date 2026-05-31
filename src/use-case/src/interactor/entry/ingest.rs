use crate::dto::entry::ingest::IngestEntryResponse;
use crate::entry_parser::EntryParser;
use crate::error::Result;
use crate::interactor::entry::EntryInteractor;
use crate::pdf_fetcher::PdfFetcher;
use crate::repository::Repository;

impl<R: Repository, E: EntryParser, F: PdfFetcher> EntryInteractor<R, E, F> {
    pub async fn ingest_entry_pdf(&self, source: &str) -> Result<IngestEntryResponse> {
        let bytes = if source.starts_with("http://") || source.starts_with("https://") {
            self.fetcher.fetch(source)?
        } else {
            std::fs::read(source).map_err(|e| {
                crate::Error::InvalidArgument(format!("failed to read {source}: {e}"))
            })?
        };
        let cards = self.entry_parser.parse(&bytes)?;
        let mut cards_saved = 0;
        let mut entries_saved = 0;
        for card in &cards {
            self.repository.save_race_card(card).await?;
            cards_saved += 1;
            entries_saved += card.entries.len();
        }
        Ok(IngestEntryResponse {
            cards_saved,
            entries_saved,
        })
    }
}
