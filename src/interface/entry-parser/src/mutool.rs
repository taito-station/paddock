use std::io::Write;
use std::process::Command;

use crate::error::{Error, Result};

/// Run `mutool draw -F stext.json` against the given PDF bytes and return the JSON output.
/// stext.json preserves x/y bbox per text line, which is required to handle the multi-column
/// race-card layout (4 races per page, with odd/even horse-num split into sub-columns).
pub fn extract_stext_json(bytes: &[u8]) -> Result<String> {
    let dir = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pdf_path = dir.join(format!("paddock-entry-{pid}-{nanos}.pdf"));
    let out_path = dir.join(format!("paddock-entry-{pid}-{nanos}.json"));
    {
        let mut f = std::fs::File::create(&pdf_path)?;
        f.write_all(bytes)?;
    }
    let result = run_mutool(&pdf_path, &out_path);
    let _ = std::fs::remove_file(&pdf_path);
    let _ = std::fs::remove_file(&out_path);
    result
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
