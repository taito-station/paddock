use crate::error::{Error, Result};

/// Outcome of probing a URL with [`PdfFetcher::fetch_if_exists`]. Distinguishes a
/// present PDF from an absent one while keeping the HTTP status that made it
/// absent, so the fetch layer can record a retryable failure (#170 / ADR0024
/// 論点1): JRA answers a never-existing report with 403 and a not-yet-published
/// one with 404, and a 403 can also be a transient block on a real meeting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchProbe {
    /// The PDF exists; carries its bytes.
    Found(Vec<u8>),
    /// No PDF at this URL; carries the HTTP status (403 or 404) that said so.
    Absent(u16),
}

pub trait PdfFetcher: Send + Sync {
    fn fetch(&self, url: &str) -> Result<Vec<u8>>;

    /// Fetch a URL, returning [`FetchProbe::Absent`] with the HTTP status when the
    /// resource does not exist (HTTP 404 or 403 — JRA answers a never-existing
    /// report with 403 and a not-yet-published one with 404). Network/other errors
    /// are still surfaced as `Err`.
    ///
    /// Used by meeting-day discovery to probe whether a result PDF exists without
    /// treating "not published yet" as a hard failure, while keeping the status so
    /// a boundary absence can be persisted as a retryable `failed` row.
    fn fetch_if_exists(&self, url: &str) -> Result<FetchProbe>;
}

/// PDF を扱わない bin 向けの no-op スタブ（#410・[`crate::pdf_parser::NoopParser`] と対）。
/// これらの bin は PDF 系ユースケースを呼ばないため `fetch` / `fetch_if_exists` は決して実行されない。
/// 誤って呼ばれた場合はサイレントな空成功にせず `InvalidArgument` で明示的に失敗させる
/// （従来 5 apps が各自定義していた `UnusedFetcher` を集約）。
pub struct NoopFetcher;

impl PdfFetcher for NoopFetcher {
    fn fetch(&self, _url: &str) -> Result<Vec<u8>> {
        Err(Error::InvalidArgument(
            "this binary does not fetch PDFs".into(),
        ))
    }

    fn fetch_if_exists(&self, _url: &str) -> Result<FetchProbe> {
        Err(Error::InvalidArgument(
            "this binary does not fetch PDFs".into(),
        ))
    }
}
