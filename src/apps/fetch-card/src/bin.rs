mod cli;
mod setup;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    let (netkeiba_id, race_id) = args.resolve_race_id()?;

    let app = setup::build_app(args.interval).await?;
    let resp = app
        .card
        .ingest(&netkeiba_id, race_id.clone(), args.force)
        .await?;

    if resp.card_saved {
        println!(
            "出馬表: {} 頭を保存（race_id={}, netkeiba={}）",
            resp.entries_saved, race_id, netkeiba_id
        );
    } else {
        println!("出馬表: 取得済みのためスキップ（--force で再取得）");
    }
    if resp.odds_saved > 0 {
        println!("オッズ: {} 件を保存（単複＋馬連・馬単・三連複・三連単）", resp.odds_saved);
    } else {
        println!("オッズ: 未確定のため保存なし");
    }

    if args.skip_history {
        println!("近走: --skip-history のため取り込みなし");
    } else {
        run_history(&app, &netkeiba_id, &resp.horse_ids).await?;
    }
    Ok(())
}

/// 出走各馬の過去走を取り込み、予想の馬個体 factor（recent_form / horse_stats）を生かす（#103）。
/// card 取得とは独立に毎回走る（--force 不要）。
async fn run_history(
    app: &setup::App,
    netkeiba_id: &str,
    horse_ids: &[String],
) -> anyhow::Result<()> {
    // card 取得時に採れた horse_id があればそれを直接使い、同じ出馬表ページの再取得を避ける（#103）。
    // 取得済みスキップ等で horse_id が空のときのみ、race_id から出馬表を引いて horse_id を集める。
    let hist = if horse_ids.is_empty() {
        let netkeiba_ids = [netkeiba_id.to_owned()];
        app.history.fetch_and_store(&netkeiba_ids, &[]).await?
    } else {
        app.history.fetch_and_store(&[], horse_ids).await?
    };
    println!(
        "近走: {} 頭（失敗 {} 頭） / 保存: {} 近走",
        hist.horses_fetched, hist.horses_failed, hist.runs_saved
    );
    // 近走取り込みは card/オッズ（本コマンドの主目的）に対する best-effort の補完。
    // shutuba 取得が失敗しても警告のみで継続し、終了コードは 0 のままにする
    // （card/オッズ保存まで成功している実行を history 失敗で巻き添えにしない）。
    if hist.shutuba_failed > 0 {
        eprintln!(
            "警告: 出馬表 {} 件の取得に失敗（対象馬が未取得）。ログを確認してください",
            hist.shutuba_failed
        );
    }
    // 取得で horses マスタが更新された直後に pdf 成績行の horse_id を埋める（fetch-history と同じ後処理）。
    let filled = app.history.backfill_horse_ids().await?;
    println!("horse_id 紐付け: {filled} 行");
    Ok(())
}
