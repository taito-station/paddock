mod cli;
mod setup;

use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Parser;

const INBOX_DIR: &str = "pdfs/inbox";
const DONE_DIR: &str = "pdfs/done";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    let app = setup::build_app().await?;
    let response = app.ingest_pdf(&args.source).await?;
    println!(
        "ingested: {} race(s), {} horse result(s) from {}",
        response.races_saved, response.horses_saved, args.source
    );
    if let Some(dest) = move_to_done_if_inbox(&args.source)? {
        println!("moved: {} -> {}", args.source, dest.display());
    }
    Ok(())
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
