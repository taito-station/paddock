mod cli;
mod session;
mod setup;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    let app = setup::build_app().await?;
    if args.summary {
        if args.budget.is_some() {
            println!("注意: --summary では --budget は無視されます。");
        }
        session::print_session_summary(&app, args.date).await?;
    } else if args.settle {
        if args.budget.is_some() {
            println!("注意: --settle では --budget は無視されます。");
        }
        session::run_settle(&app, args.date).await?;
    } else {
        session::run_session(&app, args.date, args.budget, args.race_budget, args.resume).await?;
    }
    Ok(())
}
