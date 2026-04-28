mod cli;
mod setup;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use paddock_use_case::dto::pdf::ingest::IngestPdfResponse;
use paddock_use_case::util::is_http_url;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    let parallel = resolve_parallel(args.parallel);
    let app = Arc::new(setup::build_app().await?);
    let total = args.sources.len();
    let semaphore = Arc::new(Semaphore::new(parallel));
    let mut joinset: JoinSet<(String, paddock_use_case::Result<IngestPdfResponse>)> =
        JoinSet::new();

    for source in args.sources {
        let app = Arc::clone(&app);
        let semaphore = Arc::clone(&semaphore);
        joinset.spawn(async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .expect("semaphore is never closed");
            let result = app.ingest_pdf(&source).await;
            (source, result)
        });
    }

    let mut succeeded = 0usize;
    let mut failed = 0usize;
    while let Some(joined) = joinset.join_next().await {
        let (source, result) = joined?;
        match result {
            Ok(response) => {
                println!(
                    "ingested: {} race(s), {} horse result(s) from {}",
                    response.races_saved, response.horses_saved, source
                );
                match move_to_done_if_inbox(&source) {
                    Ok(Some(dest)) => println!("moved: {} -> {}", source, dest.display()),
                    Ok(None) => {}
                    Err(e) => eprintln!(
                        "warn: failed to move {source}: {e} (re-running is safe; ingestion is idempotent)"
                    ),
                }
                succeeded += 1;
            }
            Err(e) => {
                eprintln!("error: failed to ingest {source}: {e}");
                failed += 1;
            }
        }
    }

    println!("---");
    println!("done: {succeeded}/{total} succeeded (parallel={parallel})");

    if failed > 0 {
        anyhow::bail!("{failed} of {total} files failed");
    }
    Ok(())
}

fn resolve_parallel(requested: Option<usize>) -> usize {
    requested
        .or_else(|| std::thread::available_parallelism().ok().map(|n| n.get()))
        .unwrap_or(4)
        .max(1)
}

/// `<root>/pdfs/inbox/<file>` 形式のローカルパスのみ `<root>/pdfs/done/<file>` へ
/// 移動する。判定は src 自身の親ディレクトリ構造で行うので CWD に依存しない。
fn move_to_done_if_inbox(source: &str) -> anyhow::Result<Option<PathBuf>> {
    if is_http_url(source) {
        return Ok(None);
    }
    let src = Path::new(source);
    let Ok(canonical_src) = src.canonicalize() else {
        return Ok(None);
    };
    let Some(parent) = canonical_src.parent() else {
        return Ok(None);
    };
    let Some(grandparent) = parent.parent() else {
        return Ok(None);
    };
    if parent.file_name().and_then(|n| n.to_str()) != Some("inbox") {
        return Ok(None);
    }
    if grandparent.file_name().and_then(|n| n.to_str()) != Some("pdfs") {
        return Ok(None);
    }

    let file_name = canonical_src
        .file_name()
        .context("source has no file name")?;
    let done = grandparent.join("done");
    std::fs::create_dir_all(&done).with_context(|| format!("create {}", done.display()))?;
    let dest = done.join(file_name);
    std::fs::rename(&canonical_src, &dest)
        .with_context(|| format!("rename {} -> {}", canonical_src.display(), dest.display()))?;
    Ok(Some(dest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn resolve_parallel_clamps_zero_to_one() {
        assert_eq!(resolve_parallel(Some(0)), 1);
    }

    #[test]
    fn resolve_parallel_uses_explicit_value() {
        assert_eq!(resolve_parallel(Some(8)), 8);
    }

    #[test]
    fn resolve_parallel_falls_back_to_available_parallelism() {
        let n = resolve_parallel(None);
        assert!(n >= 1);
    }

    #[test]
    fn move_skips_url_sources() {
        assert!(matches!(
            move_to_done_if_inbox("https://example.com/a.pdf"),
            Ok(None)
        ));
        assert!(matches!(
            move_to_done_if_inbox("http://example.com/a.pdf"),
            Ok(None)
        ));
    }

    #[test]
    fn move_skips_files_outside_pdfs_inbox() {
        let dir = tempdir().unwrap();
        let pdf = dir.path().join("foo.pdf");
        fs::write(&pdf, b"x").unwrap();

        let result = move_to_done_if_inbox(pdf.to_str().unwrap()).unwrap();

        assert!(result.is_none());
        assert!(pdf.exists(), "src must remain when not under pdfs/inbox");
    }

    #[test]
    fn move_skips_inbox_outside_pdfs_root() {
        let dir = tempdir().unwrap();
        let inbox = dir.path().join("other/inbox");
        fs::create_dir_all(&inbox).unwrap();
        let pdf = inbox.join("foo.pdf");
        fs::write(&pdf, b"x").unwrap();

        let result = move_to_done_if_inbox(pdf.to_str().unwrap()).unwrap();

        assert!(result.is_none());
        assert!(pdf.exists());
    }

    #[test]
    fn move_relocates_pdfs_inbox_file_to_done() {
        let dir = tempdir().unwrap();
        let inbox = dir.path().join("pdfs/inbox");
        fs::create_dir_all(&inbox).unwrap();
        let src = inbox.join("foo.pdf");
        fs::write(&src, b"x").unwrap();

        let dest = move_to_done_if_inbox(src.to_str().unwrap())
            .unwrap()
            .expect("inbox file should be moved");

        assert_eq!(dest.file_name().unwrap(), "foo.pdf");
        assert_eq!(dest.parent().unwrap().file_name().unwrap(), "done");
        assert!(!src.exists(), "src should be moved away");
        assert!(dest.exists(), "dest should exist after move");
    }
}
