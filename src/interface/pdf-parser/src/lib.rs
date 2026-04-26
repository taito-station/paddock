pub mod error;
pub mod extract;
pub mod fetcher;
pub mod hybrid;
pub mod parser;

pub use error::{Error, Result};
pub use fetcher::UreqFetcher;
pub use hybrid::HybridParser;
pub use parser::MutoolParser;
