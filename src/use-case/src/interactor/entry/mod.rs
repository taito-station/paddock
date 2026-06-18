pub mod ingest;

use crate::entry_parser::EntryParser;
use crate::pdf_fetcher::PdfFetcher;

pub struct EntryInteractor<R, E: EntryParser, F: PdfFetcher> {
    pub repository: R,
    pub entry_parser: E,
    pub fetcher: F,
}

impl<R, E: EntryParser, F: PdfFetcher> EntryInteractor<R, E, F> {
    pub fn new(repository: R, entry_parser: E, fetcher: F) -> Self {
        Self {
            repository,
            entry_parser,
            fetcher,
        }
    }
}
