use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

use crate::error::{Error, Result};

pub struct RenderedPage {
    pub page_num: u32,
    pub png_path: PathBuf,
}

/// Holds rendered PNG files in a temp directory. Pages are dropped when this struct is dropped.
pub struct RenderedPdf {
    _tmp: TempDir,
    pub pages: Vec<RenderedPage>,
}

/// Render every page of the PDF as a PNG using `mutool draw -F png`.
/// `dpi` controls resolution; 200 is a reasonable default for OCR on JRA layouts.
pub fn render_pdf_to_pngs(pdf_bytes: &[u8], dpi: u32) -> Result<RenderedPdf> {
    let tmp = TempDir::new()?;
    let pdf_path = tmp.path().join("input.pdf");
    std::fs::write(&pdf_path, pdf_bytes)?;
    let pattern = tmp.path().join("page-%d.png");

    let output = Command::new("mutool")
        .args([
            "draw",
            "-F",
            "png",
            "-r",
            &dpi.to_string(),
            "-o",
            pattern
                .to_str()
                .ok_or_else(|| Error::Mutool("output path contains invalid utf-8".to_string()))?,
            pdf_path
                .to_str()
                .ok_or_else(|| Error::Mutool("input path contains invalid utf-8".to_string()))?,
        ])
        .output()?;
    if !output.status.success() {
        return Err(Error::Mutool(format!(
            "mutool exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let mut pages = Vec::new();
    let mut page_num: u32 = 1;
    loop {
        let path = tmp.path().join(format!("page-{page_num}.png"));
        if !path.exists() {
            break;
        }
        pages.push(RenderedPage {
            page_num,
            png_path: path,
        });
        page_num += 1;
    }
    if pages.is_empty() {
        return Err(Error::Mutool("mutool produced no output pages".to_string()));
    }
    Ok(RenderedPdf { _tmp: tmp, pages })
}
