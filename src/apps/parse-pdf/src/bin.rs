mod cli;
mod setup;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use paddock_use_case::dto::pdf::ingest::IngestPdfResponse;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::Instrument;

const INBOX_DIR: &str = "pdfs/inbox";
const DONE_DIR: &str = "pdfs/done";

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
        let permit = Arc::clone(&semaphore)
            .acquire_owned()
            .await
            .context("acquire semaphore permit")?;
        let span = tracing::info_span!("ingest", source = %source);
        joinset.spawn(
            async move {
                let _permit = permit;
                let result = app.ingest_pdf(&source).await;
                (source, result)
            }
            .instrument(span),
        );
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

fn move_to_done_if_inbox(source: &str) -> anyhow::Result<Option<PathBuf>> {
    if source.starts_with("http://") || source.starts_with("https://") {
        return Ok(None);
    }
    let src = Path::new(source);
    let (Ok(canonical_src), Ok(canonical_inbox)) =
        (src.canonicalize(), Path::new(INBOX_DIR).canonicalize())
    else {
        return Ok(None);
    };
    if !canonical_src.starts_with(&canonical_inbox) {
        return Ok(None);
    }
    let file_name = src.file_name().context("source has no file name")?;
    let done = Path::new(DONE_DIR);
    std::fs::create_dir_all(done).with_context(|| format!("create {}", done.display()))?;
    let dest = done.join(file_name);
    std::fs::rename(src, &dest)
        .with_context(|| format!("rename {} -> {}", src.display(), dest.display()))?;
    Ok(Some(dest))
}
