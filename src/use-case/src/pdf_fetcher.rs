use crate::error::Result;

pub trait PdfFetcher: Send + Sync {
    fn fetch(&self, url: &str) -> Result<Vec<u8>>;
}
