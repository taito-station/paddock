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
        read_body(resp)
    }

    fn fetch_if_exists(&self, url: &str) -> UcResult<Option<Vec<u8>>> {
        match ureq::get(url).call() {
            Ok(resp) => Ok(Some(read_body(resp)?)),
            // 404 means the meeting PDF is not published (yet); treat as absent.
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => Err(Error::Fetch(e.to_string()).into()),
        }
    }
}

fn read_body(resp: ureq::Response) -> UcResult<Vec<u8>> {
    let mut buf = Vec::new();
    resp.into_reader()
        .read_to_end(&mut buf)
        .map_err(Error::Io)?;
    Ok(buf)
}
