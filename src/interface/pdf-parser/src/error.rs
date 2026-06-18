use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("mutool failed: {0}")]
    Mutool(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ocr error: {0}")]
    Ocr(#[from] pdf_ocr::Error),
    #[error("domain error: {0}")]
    Domain(#[from] paddock_domain::Error),
}

pub type Result<A> = std::result::Result<A, Error>;

impl From<Error> for paddock_use_case::Error {
    fn from(value: Error) -> Self {
        // Every parser failure — including `Io` — maps to `Internal` on purpose:
        // these are parse-time *local* failures (mutool exec, temp files, glyph
        // decoding), genuinely internal to this crate. This is intentionally
        // distinct from `jra-fetcher`, where a network body-read maps to
        // `Error::Fetch`. The asymmetry reflects the error's origin (local vs
        // network), not an oversight.
        paddock_use_case::Error::Internal(value.to_string())
    }
}
