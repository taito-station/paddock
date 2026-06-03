pub mod error;
pub mod parse;
pub mod scraper;

pub use error::{Error, Result};
pub use scraper::{OddsPages, UreqOddsScraper, assemble};
