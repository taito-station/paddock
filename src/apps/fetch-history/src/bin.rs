mod cli;
mod setup;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    if args.race_ids.is_empty() && args.horse_ids.is_empty() {
        anyhow::bail!("race_id か --horse-id を 1 つ以上指定してください");
    }

    let app = setup::build_app().await?;
    let resp = app.fetch_and_store(&args.race_ids, &args.horse_ids).await?;

    println!(
        "取得: {} 頭（失敗 {} 頭） / 保存: {} レース・{} 近走",
        resp.horses_fetched, resp.horses_failed, resp.races_saved, resp.results_saved
    );
    if resp.shutuba_failed > 0 {
        // 出馬表取得失敗はその出走馬が丸ごと欠落するため、件数を明示する。
        eprintln!(
            "警告: 出馬表 {} 件の取得に失敗（対象馬が未取得）。ログを確認してください",
            resp.shutuba_failed
        );
    }
    Ok(())
}
