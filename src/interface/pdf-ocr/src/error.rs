use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("mutool: {0}")]
    Mutool(String),

    #[error("tesseract: {0}")]
    Tesseract(String),

    #[error("parse: {0}")]
    Parse(String),
}

pub type Result<A> = std::result::Result<A, Error>;
