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
        // race_id 導出に失敗しても当該レースだけスキップしてバッチは継続する（fetch 失敗と同様）。
        let netkeiba_id = match build_race_ids(year, race.venue, race.round, race.day, race.race_num)
        {
            Ok((id, _)) => id,
            Err(e) => {
                failed += 1;
                tracing::warn!(race = %race.race_id, "netkeiba race_id 導出失敗、スキップ: {e}");
                continue;
            }
        };
        let rows = match scraper.fetch_race_result(&netkeiba_id) {
            Ok(rows) => rows,
            Err(e) => {
                failed += 1;
                tracing::warn!(race = %race.race_id, netkeiba = %netkeiba_id, "結果取得失敗: {e}");
                continue;
            }
        };
        // DB 更新失敗も取得失敗と同様に当該レースのみ skip し、バッチ完走性を揃える。
        match repo.update_results(&race.race_id, &rows).await {
            Ok(n) => {
                updated_total += n;
                ok += 1;
                // 取得頭数と実更新行数の乖離（horse_num 不一致・レース未取込）を可視化する。
                if (n as usize) < rows.len() {
                    tracing::warn!(
                        race = %race.race_id,
                        fetched = rows.len(),
                        updated = n,
                        "一部の馬番が既存 results に一致せず未更新"
                    );
                }
            }
            Err(e) => {
                failed += 1;
                tracing::warn!(race = %race.race_id, "results 更新失敗: {e}");
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
