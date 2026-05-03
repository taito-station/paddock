use crate::dto::pdf::ingest::IngestPdfResponse;
use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::Repository;

impl<R: Repository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    pub async fn ingest_pdf(&self, source: &str) -> Result<IngestPdfResponse> {
        let span = tracing::info_span!("ingest", source = %source);
        let _enter = span.enter();
        let bytes = if source.starts_with("http://") || source.starts_with("https://") {
            self.pdf_fetcher.fetch(source)?
        } else {
            std::fs::read(source).map_err(|e| {
                crate::Error::InvalidArgument(format!("failed to read {source}: {e}"))
            })?
        };
        let races = self.pdf_parser.parse(&bytes)?;
        let mut races_saved = 0;
        let mut horses_saved = 0;
        for race in &races {
            self.repository.save_race(race).await?;
            races_saved += 1;
            horses_saved += race.results.len();
        }
        Ok(IngestPdfResponse {
            races_saved,
            horses_saved,
        })
    }
}
