use paddock_domain::Race;

use crate::error::Result;

pub trait PdfParser: Send + Sync {
    fn parse(&self, bytes: &[u8]) -> Result<Vec<Race>>;
}
