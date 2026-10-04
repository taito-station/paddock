use clap::Parser;

mod cli;
mod collect;
mod setup;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    let app = setup::build_app(args.scrape_delay).await?;
    // pool を閉じてから終える（#717）
    rdb_gateway::pool::close_after(&app.pool, collect::run(&app, &args)).await
}
