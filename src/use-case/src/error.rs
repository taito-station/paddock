use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<A> = std::result::Result<A, Error>;

impl From<paddock_domain::Error> for Error {
    fn from(value: paddock_domain::Error) -> Self {
        Error::InvalidArgument(value.to_string())
    }
}
