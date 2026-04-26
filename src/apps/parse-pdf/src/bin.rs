mod cli;
mod setup;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    let app = setup::build_app().await?;
    let response = app.ingest_pdf(&args.source).await?;
    println!(
        "ingested: {} race(s), {} horse result(s) from {}",
        response.races_saved, response.horses_saved, args.source
    );
    Ok(())
}
