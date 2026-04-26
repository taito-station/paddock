use std::io::Write;
use std::process::Command;

use paddock_domain::Race;
use paddock_use_case::Result as UcResult;
use paddock_use_case::pdf_parser::PdfParser;

use crate::error::{Error, Result};
use crate::extract;

pub struct MutoolParser;

impl PdfParser for MutoolParser {
    fn parse(&self, bytes: &[u8]) -> UcResult<Vec<Race>> {
        let text = mutool_extract(bytes).map_err(paddock_use_case::Error::from)?;
        let races = extract::parse_text(&text).map_err(paddock_use_case::Error::from)?;
        Ok(races)
    }
}

fn mutool_extract(bytes: &[u8]) -> Result<String> {
    // mutool does not reliably accept PDF input on stdin, so write to a temp file.
    let dir = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pdf_path = dir.join(format!("paddock-{pid}-{nanos}.pdf"));
    {
        let mut f = std::fs::File::create(&pdf_path)?;
        f.write_all(bytes)?;
    }
    let result = run_mutool(&pdf_path);
    let _ = std::fs::remove_file(&pdf_path);
    result
}

fn run_mutool(pdf_path: &std::path::Path) -> Result<String> {
    let output = Command::new("mutool")
        .args(["draw", "-q", "-F", "text"])
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
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
