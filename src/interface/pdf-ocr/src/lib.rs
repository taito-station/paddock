mod error;
mod extract;
mod render;
mod tesseract;

pub use error::{Error, Result};
pub use extract::{OcrExtractor, OcrPageRows, OcrRow};
pub use render::{RenderedPage, render_pdf_to_pngs};
pub use tesseract::{OcrToken, ensure_available, run_tesseract};
