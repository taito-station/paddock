use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid format: {0}")]
    InvalidFormat(String),
    #[error("invalid length range: {0}")]
    InvalidLengthRange(String),
    #[error("out of range: {0}")]
    OutOfRange(String),
}

pub type Result<A> = std::result::Result<A, Error>;
