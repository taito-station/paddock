use crate::error::Result;

pub trait PdfFetcher: Send + Sync {
    fn fetch(&self, url: &str) -> Result<Vec<u8>>;

    /// Fetch a URL, returning `Ok(None)` when the resource does not exist
    /// (HTTP 404). Network/other errors are still surfaced as `Err`.
    ///
    /// Used by meeting-day discovery to probe whether a result PDF exists
    /// without treating "not published yet" as a hard failure.
    fn fetch_if_exists(&self, url: &str) -> Result<Option<Vec<u8>>>;
}
