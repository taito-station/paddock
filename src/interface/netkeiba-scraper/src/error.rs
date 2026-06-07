use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("netkeiba fetch failed: {0}")]
    Fetch(String),
    #[error("netkeiba parse failed: {0}")]
    Parse(String),
}

pub type Result<A> = std::result::Result<A, Error>;

impl From<Error> for paddock_use_case::Error {
    fn from(value: Error) -> Self {
        paddock_use_case::Error::Internal(value.to_string())
    }
}
