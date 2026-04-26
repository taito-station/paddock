mod columns;
mod row;

pub use row::{OcrPageRows, OcrRow};

use crate::error::Result;
use crate::render::render_pdf_to_pngs;
use crate::tesseract::run_tesseract;

/// High-level OCR extractor: PDF bytes → per-page rows of (horse_num, position, trainer, weight, popularity).
pub struct OcrExtractor {
    pub dpi: u32,
    pub lang: String,
}

impl Default for OcrExtractor {
    fn default() -> Self {
        Self {
            dpi: 200,
            lang: "jpn".to_string(),
        }
    }
}

impl OcrExtractor {
    pub fn extract(&self, pdf_bytes: &[u8]) -> Result<Vec<OcrPageRows>> {
        let rendered = render_pdf_to_pngs(pdf_bytes, self.dpi)?;
        let mut pages = Vec::with_capacity(rendered.pages.len());
        for page in &rendered.pages {
            let tokens = run_tesseract(&page.png_path, &self.lang)?;
            let rows = row::tokens_to_rows(page.page_num, &tokens);
            pages.push(rows);
        }
        Ok(pages)
    }
}
