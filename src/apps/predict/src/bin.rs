mod cli;
mod session;
mod setup;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    let app = setup::build_app().await?;
    session::run_session(&app, args.date, args.budget).await?;
    Ok(())
}
