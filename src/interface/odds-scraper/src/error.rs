use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("fetch error: {0}")]
    Fetch(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("domain error: {0}")]
    Domain(#[from] paddock_domain::Error),
}

pub type Result<A> = std::result::Result<A, Error>;

impl From<Error> for paddock_use_case::Error {
    fn from(value: Error) -> Self {
        match &value {
            // Malformed odds HTML / out-of-range values are input problems.
            Error::Parse(_) | Error::Domain(_) => {
                paddock_use_case::Error::InvalidArgument(value.to_string())
            }
            // Network / IO failures are infrastructure problems.
            Error::Fetch(_) => paddock_use_case::Error::Internal(value.to_string()),
        }
    }
}
