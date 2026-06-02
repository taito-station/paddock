use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("fetch error: {0}")]
    Fetch(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("domain error: {0}")]
    Domain(#[from] paddock_domain::Error),
}

pub type Result<A> = std::result::Result<A, Error>;

impl From<Error> for paddock_use_case::Error {
    fn from(value: Error) -> Self {
        paddock_use_case::Error::Internal(value.to_string())
    }
}
