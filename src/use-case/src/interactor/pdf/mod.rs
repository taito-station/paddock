pub mod fetch;
pub mod ingest;

use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;

/// JRA 成績 PDF の取得・解析ユースケース専用の facade（#453）。
/// PDF 取得（`F: PdfFetcher`）と解析（`P: PdfParser`）を要する `fetch_meeting` /
/// `fetch_meeting_range` / `ingest_pdf` をここに閉じ込め、非 PDF アプリ（predict / analyze /
/// api-server 等）の [`super::Interactor`] から PDF ジェネリクスを排除する。実運用では
/// parse-pdf アプリのみが実 Parser/Fetcher を注入して構築する。
pub struct PdfInteractor<R, P: PdfParser, F: PdfFetcher> {
    pub repository: R,
    pub pdf_parser: P,
    pub pdf_fetcher: F,
}

impl<R, P: PdfParser, F: PdfFetcher> PdfInteractor<R, P, F> {
    pub fn new(repository: R, pdf_parser: P, pdf_fetcher: F) -> Self {
        Self {
            repository,
            pdf_parser,
            pdf_fetcher,
        }
    }
}
