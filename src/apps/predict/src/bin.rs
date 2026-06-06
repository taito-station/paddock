mod cli;
mod session;
mod setup;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    let app = setup::build_app().await?;
    if args.summary {
        session::print_session_summary(&app, args.date).await?;
    } else {
        session::run_session(&app, args.date, args.budget, args.resume).await?;
    }
    Ok(())
}
