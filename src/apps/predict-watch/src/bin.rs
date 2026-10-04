mod cli;
mod notify;
mod setup;
mod snapshot;
mod watch;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    let app = setup::build_app(args.scrape_delay).await?;
    // pool を閉じてから終える（#717）
    rdb_gateway::pool::close_after(&app.pool, watch::run(&app, &args)).await
}
