mod cli;
mod setup;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use paddock_use_case::dto::entry::ingest::IngestEntryResponse;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    let parallel = resolve_parallel(args.parallel);
    let app = Arc::new(setup::build_app().await?);
    let total = args.sources.len();
    let semaphore = Arc::new(Semaphore::new(parallel));
    let mut joinset: JoinSet<(String, paddock_use_case::Result<IngestEntryResponse>)> =
        JoinSet::new();

    for source in args.sources {
        let app = Arc::clone(&app);
        let permit = Arc::clone(&semaphore)
            .acquire_owned()
            .await
            .context("acquire semaphore permit")?;
        joinset.spawn(async move {
            let _permit = permit;
            let result = app.ingest_entry_pdf(&source).await;
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
                    "ingested: {} race card(s), {} horse entry/entries from {}",
                    response.cards_saved, response.entries_saved, source
                );
                match move_to_done_if_inbox(&source) {
                    Ok(Some(dest)) => println!("moved: {} -> {}", source, dest.display()),
                    Ok(None) => {}
                    Err(e) => eprintln!("warn: failed to move {source}: {e}"),
                }
                succeeded += 1;
            }
            Err(e) => {
                eprintln!("error: failed to ingest {source}: {e}");
                failed += 1;
            }
        }
    }

    if total > 1 {
        println!("---");
        println!("done: {succeeded}/{total} succeeded (parallel={parallel})");
    }

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

/// Move a successfully ingested file from `<root>/pdfs/<kind>/inbox/<file>` to
/// `<root>/pdfs/<kind>/done/<file>`.
fn move_to_done_if_inbox(source: &str) -> anyhow::Result<Option<PathBuf>> {
    if source.starts_with("http://") || source.starts_with("https://") {
        return Ok(None);
    }
    let src = Path::new(source);
    let Ok(canonical_src) = src.canonicalize() else {
        return Ok(None);
    };
    let Some(parent) = canonical_src.parent() else {
        return Ok(None);
    };
    if parent.file_name().and_then(|n| n.to_str()) != Some("inbox") {
        return Ok(None);
    }
    let Some(kind_dir) = parent.parent() else {
        return Ok(None);
    };
    let Some(pdfs_dir) = kind_dir.parent() else {
        return Ok(None);
    };
    if pdfs_dir.file_name().and_then(|n| n.to_str()) != Some("pdfs") {
        return Ok(None);
    }
    let file_name = canonical_src
        .file_name()
        .context("source has no file name")?;
    let done = kind_dir.join("done");
    std::fs::create_dir_all(&done).with_context(|| format!("create {}", done.display()))?;
    let dest = done.join(file_name);
    std::fs::rename(&canonical_src, &dest)
        .with_context(|| format!("rename {} -> {}", canonical_src.display(), dest.display()))?;
    Ok(Some(dest))
}
