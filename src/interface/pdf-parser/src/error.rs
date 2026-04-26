use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("mutool failed: {0}")]
    Mutool(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("fetch error: {0}")]
    Fetch(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("domain error: {0}")]
    Domain(#[from] paddock_domain::Error),
}

pub type Result<A> = std::result::Result<A, Error>;

impl From<Error> for paddock_use_case::Error {
    fn from(value: Error) -> Self {
        match &value {
            Error::Fetch(_) | Error::Io(_) => paddock_use_case::Error::Internal(value.to_string()),
            _ => paddock_use_case::Error::Internal(value.to_string()),
        }
    }
}
