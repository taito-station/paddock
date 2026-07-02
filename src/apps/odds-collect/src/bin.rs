use clap::Parser;

mod cli;
mod collect;
mod setup;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    let app = setup::build_app(args.scrape_delay).await?;
    collect::run(&app, &args).await
}
