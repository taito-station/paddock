use std::io::Write;
use std::process::Command;

use crate::error::{Error, Result};

/// Run `mutool draw -F stext.json` against the given PDF bytes and return the JSON output.
/// stext.json preserves x/y bbox per text line, which is required to handle the multi-column
/// race-card layout (4 races per page, with odd/even horse-num split into sub-columns).
///
/// Temp files are created via the `tempfile` crate so that (a) names are guaranteed unique even
/// when several PDFs are ingested in parallel, and (b) they are removed on drop even if an early
/// return occurs.
pub fn extract_stext_json(bytes: &[u8]) -> Result<String> {
    let mut pdf = tempfile::Builder::new()
        .prefix("paddock-entry-")
        .suffix(".pdf")
        .tempfile()?;
    pdf.write_all(bytes)?;
    pdf.flush()?;

    let out = tempfile::Builder::new()
        .prefix("paddock-entry-")
        .suffix(".json")
        .tempfile()?;

    run_mutool(pdf.path(), out.path())
}

fn run_mutool(pdf_path: &std::path::Path, out_path: &std::path::Path) -> Result<String> {
    let output = Command::new("mutool")
        .args(["draw", "-q", "-F", "stext.json", "-o"])
        .arg(out_path)
        .arg(pdf_path)
        .output()
        .map_err(|e| {
            Error::Mutool(format!(
                "failed to spawn `mutool` (is it installed? brew install mupdf-tools): {e}"
            ))
        })?;
    if !output.status.success() {
        return Err(Error::Mutool(format!(
            "mutool exited with status {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    std::fs::read_to_string(out_path).map_err(Error::Io)
}
