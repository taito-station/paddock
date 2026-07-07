mod cli;
mod setup;
mod snapshot;
mod watch;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    let app = setup::build_app(args.scrape_delay).await?;
    watch::run(&app, &args).await
}
