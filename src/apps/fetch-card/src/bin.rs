mod cli;
mod setup;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    let (netkeiba_id, race_id) = args.resolve_race_id()?;

    let app = setup::build_app(args.interval).await?;
    let resp = app.ingest(&netkeiba_id, race_id.clone(), args.force).await?;

    if resp.card_saved {
        println!(
            "出馬表: {} 頭を保存（race_id={}, netkeiba={}）",
            resp.entries_saved, race_id, netkeiba_id
        );
    } else {
        println!("出馬表: 取得済みのためスキップ（--force で再取得）");
    }
    if resp.odds_saved > 0 {
        println!("単勝オッズ: {} 件を保存", resp.odds_saved);
    } else {
        println!("単勝オッズ: 未確定のため保存なし");
    }
    Ok(())
}
