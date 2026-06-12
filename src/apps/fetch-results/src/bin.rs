mod cli;
mod setup;

use chrono::Datelike;
use clap::Parser;
use paddock_use_case::build_race_ids;
use paddock_use_case::repository::Repository;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    let (repo, scraper) = setup::build(args.interval).await?;

    // 既存の確定済み(pdf)レースを対象に、netkeiba 結果で results を差し替える。
    let races = repo.find_finished_races_between(args.from, args.to).await?;
    let total = races.len();
    println!(
        "対象 {total} レースを netkeiba 結果で再取込します ({} 〜 {})",
        args.from, args.to
    );

    let mut ok = 0usize;
    let mut failed = 0usize;
    let mut updated_total = 0u64;
    for (i, race) in races.iter().enumerate() {
        let year = race.date.year() as u32;
        let (netkeiba_id, _) =
            build_race_ids(year, race.venue, race.round, race.day, race.race_num)?;
        match scraper.fetch_race_result(&netkeiba_id) {
            Ok(rows) => {
                let n = repo.update_results(&race.race_id, &rows).await?;
                updated_total += n;
                ok += 1;
            }
            Err(e) => {
                failed += 1;
                tracing::warn!(race = %race.race_id, netkeiba = %netkeiba_id, "結果取得失敗: {e}");
            }
        }
        if (i + 1) % 20 == 0 || i + 1 == total {
            println!(
                "[{}/{}] 更新 {updated_total} 行 (成功 {ok} / 失敗 {failed})",
                i + 1,
                total
            );
        }
    }
    println!("完了: 成功 {ok} / 失敗 {failed} / 計 {updated_total} 行更新");
    Ok(())
}
