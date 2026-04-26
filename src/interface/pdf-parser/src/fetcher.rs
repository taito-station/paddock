use std::io::Read;

use paddock_use_case::Result as UcResult;
use paddock_use_case::pdf_fetcher::PdfFetcher;

use crate::error::Error;

pub struct UreqFetcher;

impl PdfFetcher for UreqFetcher {
    fn fetch(&self, url: &str) -> UcResult<Vec<u8>> {
        let resp = ureq::get(url)
            .call()
            .map_err(|e| Error::Fetch(e.to_string()))?;
        let mut buf = Vec::new();
        resp.into_reader().read_to_end(&mut buf).map_err(Error::Io)?;
        Ok(buf)
    }
}
