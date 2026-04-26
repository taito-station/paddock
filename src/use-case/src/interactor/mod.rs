pub mod course;
pub mod horse;
pub mod jockey;
pub mod pdf;

use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::Repository;

pub struct Interactor<R: Repository, P: PdfParser, F: PdfFetcher> {
    pub repository: R,
    pub pdf_parser: P,
    pub pdf_fetcher: F,
}

impl<R: Repository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    pub fn new(repository: R, pdf_parser: P, pdf_fetcher: F) -> Self {
        Self {
            repository,
            pdf_parser,
            pdf_fetcher,
        }
    }
}
