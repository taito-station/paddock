// lib（src/lib.rs）側の実体を使う。`mod` で再宣言すると同じソースが lib と bin で
// 二重コンパイルされ、unit test も 2 回走ってしまう（#555）。
use predict::{cli, session, setup};

use clap::Parser;
use rdb_gateway::pool;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    let app = setup::build_app().await?;
    // pool を閉じてから終える（#717）
    pool::close_after(&app.pool, run(&app, args)).await
}

async fn run(app: &setup::App, args: cli::Cli) -> anyhow::Result<()> {
    if args.overview {
        if args.budget.is_some() {
            println!("注意: --overview では --budget は無視されます（予算は --race-budget）。");
        }
        session::run_overview(app, args.date, args.race_budget, args.explain).await?;
    } else if args.summary {
        if args.budget.is_some() {
            println!("注意: --summary では --budget は無視されます。");
        }
        session::print_session_summary(app, args.date).await?;
    } else if args.settle {
        if args.budget.is_some() {
            println!("注意: --settle では --budget は無視されます。");
        }
        session::run_settle(app, args.date).await?;
    } else {
        session::run_session(
            app,
            args.date,
            args.budget,
            args.race_budget,
            args.resume,
            args.explain,
            args.skip_all,
        )
        .await?;
    }
    Ok(())
}
