use std::path::Path;
use std::process::Command;

use crate::error::{Error, Result};

/// Verify that the `tesseract` binary is available and that the requested language pack
/// (typically `"jpn"`) is installed. Intended to be called once at startup so the program
/// fails fast with a clear message instead of warning per-PDF later.
pub fn ensure_available(lang: &str) -> Result<()> {
    let version = Command::new("tesseract")
        .arg("--version")
        .output()
        .map_err(|e| {
            Error::Tesseract(format!(
                "tesseract binary not found: {e} (install via `brew install tesseract` on macOS)"
            ))
        })?;
    if !version.status.success() {
        return Err(Error::Tesseract(format!(
            "tesseract --version exited with {}",
            version.status
        )));
    }
    let langs = Command::new("tesseract")
        .arg("--list-langs")
        .output()
        .map_err(|e| Error::Tesseract(format!("tesseract --list-langs failed: {e}")))?;
    if !langs.status.success() {
        return Err(Error::Tesseract(format!(
            "tesseract --list-langs exited with {}",
            langs.status
        )));
    }
    // tesseract prints the language list on stdout, but older builds emit it on stderr —
    // scan both streams to be safe.
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&langs.stdout),
        String::from_utf8_lossy(&langs.stderr)
    );
    if !combined.lines().any(|l| l.trim() == lang) {
        return Err(Error::Tesseract(format!(
            "tesseract language pack '{lang}' not found (install via `brew install tesseract-lang` on macOS)"
        )));
    }
    Ok(())
}

/// One token from tesseract's TSV output (one word at given page coordinates).
#[derive(Debug, Clone)]
pub struct OcrToken {
    pub text: String,
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub conf: i32,
}

/// Run `tesseract <png> stdout -l <lang> --psm 6 tsv` and parse the TSV output.
/// `lang` should typically be `"jpn"` for JRA result tables.
pub fn run_tesseract(png_path: &Path, lang: &str) -> Result<Vec<OcrToken>> {
    let output = Command::new("tesseract")
        .arg(png_path)
        .arg("stdout")
        .arg("-l")
        .arg(lang)
        .arg("--psm")
        .arg("6")
        .arg("tsv")
        .output()?;
    if !output.status.success() {
        return Err(Error::Tesseract(format!(
            "tesseract exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    parse_tsv(&String::from_utf8_lossy(&output.stdout))
}

/// Parse tesseract's TSV. The header is: level page_num block_num par_num line_num word_num
/// left top width height conf text.
fn parse_tsv(tsv: &str) -> Result<Vec<OcrToken>> {
    let mut tokens = Vec::new();
    for (idx, line) in tsv.lines().enumerate() {
        if idx == 0 || line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 12 {
            continue;
        }
        let level: i32 = cols[0].parse().map_err(|_| {
            Error::Parse(format!(
                "invalid tesseract level on line {idx}: {}",
                cols[0]
            ))
        })?;
        // level 5 = word; ignore higher-level rows.
        if level != 5 {
            continue;
        }
        let text = cols[11].trim().to_string();
        if text.is_empty() {
            continue;
        }
        let left: i32 = cols[6].parse().unwrap_or(0);
        let top: i32 = cols[7].parse().unwrap_or(0);
        let width: i32 = cols[8].parse().unwrap_or(0);
        let height: i32 = cols[9].parse().unwrap_or(0);
        let conf: i32 = cols[10].parse().unwrap_or(-1);
        tokens.push(OcrToken {
            text,
            left,
            top,
            width,
            height,
            conf,
        });
    }
    Ok(tokens)
}
