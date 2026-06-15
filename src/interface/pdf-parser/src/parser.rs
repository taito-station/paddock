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
        let (text, stext) = mutool_extract(bytes).map_err(paddock_use_case::Error::from)?;
        // 騎手列・調教師列は stext.json の座標 + font サイズから抽出する。stext が取れなくても
        // （空文字列）各行のヒューリスティックにフォールバックして取り込みは継続する。
        let jockeys = extract::jockey_stext::parse_jockeys(&stext);
        let trainers = extract::jockey_stext::parse_trainers(&stext);
        // 斤量は CID 数字で読めるため stext 座標索引で確定する（#124、EdiF 復号不要）。
        let weights = extract::jockey_stext::parse_weights(&stext);
        let races = extract::parse_text(&text, &jockeys, &trainers, &weights)
            .map_err(paddock_use_case::Error::from)?;
        Ok(races)
    }
}

/// PDF を一時ファイルに書き出し、`-F text`（構造）と `-F stext.json`（座標つき、騎手抽出用）の
/// 2 形式を取得して返す。stext.json は best-effort（失敗時は空文字列）。
fn mutool_extract(bytes: &[u8]) -> Result<(String, String)> {
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
    let text = run_mutool(&pdf_path, "text");
    let stext = run_mutool(&pdf_path, "stext.json").unwrap_or_default();
    let _ = std::fs::remove_file(&pdf_path);
    Ok((text?, stext))
}

fn run_mutool(pdf_path: &std::path::Path, format: &str) -> Result<String> {
    let output = Command::new("mutool")
        .args(["draw", "-q", "-F", format])
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
