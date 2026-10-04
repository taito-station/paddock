use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use fetch_final_odds::targets::parse_targets;
use fetch_final_odds::{MIN_INTERVAL_MS, ensure_outside_repository, validate_interval};
use fill_results::{apply, fetch};
use netkeiba_scraper::UreqNetkeibaScraper;
use paddock_config::Config;
use rdb_gateway::{PostgresRepository, pool};

/// results の行が足りないレースに、足りない馬番の行だけを netkeiba の結果ページから INSERT する（#742）。
/// 既存行は消さず、上書きもしない。
#[derive(Parser, Debug)]
#[command(name = "paddock-fill-results")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// 対象レースの結果ページを取得して `<out>/raw/` に保存する（再送しない・再開可能）。DB には触れない。
    Fetch {
        /// 対象レース一覧（1 行 1 つの paddock race_id。先頭行 `race_id` はヘッダ）。
        #[arg(long)]
        targets: PathBuf,
        /// 出力先ディレクトリ。リポジトリの中は拒否する（取得データを公開リポジトリに混ぜない）。
        #[arg(long)]
        out_dir: PathBuf,
        /// リクエスト間隔（ミリ秒）。3334 未満は受け付けない。
        #[arg(long, default_value_t = 3500)]
        interval_ms: u64,
    },
    /// 保存した結果ページから、足りない馬番の行だけを INSERT する（ネットワーク不使用）。
    Apply {
        #[arg(long)]
        targets: PathBuf,
        #[arg(long)]
        out_dir: PathBuf,
        /// 書き込み先の DB 名。接続先（PADDOCK_DB_URL）の `current_database()` と違えば何もしない。
        #[arg(long)]
        db_name: String,
        /// 入れる行を表示するだけで書かない。
        #[arg(long)]
        dry_run: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env().context("load config")?;
    config.init_tracing();
    match Cli::parse().command {
        Command::Fetch {
            targets,
            out_dir,
            interval_ms,
        } => {
            validate_interval(interval_ms)?;
            let targets = parse_targets(&std::fs::read_to_string(&targets)?)?;
            // raw を先に作ってから検査する（検査の後で raw ができると、その実パスを確かめられない）。
            // 以後は検査を通った実パスだけを使う。リポジトリの中を指定して拒否されたときも、空の raw は残る（git は追跡しない）
            std::fs::create_dir_all(out_dir.join("raw"))?;
            let out_dir = ensure_outside_repository(&out_dir)?;
            println!(
                "対象 {} レース・間隔 {interval_ms}ms（下限 {MIN_INTERVAL_MS}ms）→ {}",
                targets.len(),
                out_dir.display()
            );
            let scraper = UreqNetkeibaScraper::with_delay(Duration::from_millis(interval_ms));
            let (summary, stop) = fetch::run(&targets, &scraper, &out_dir, |done, total, _| {
                println!("[{done}/{total}] 保存");
            })?;
            println!("取得 {} / スキップ {}", summary.fetched, summary.skipped);
            match fetch::exit_outcome(&stop) {
                Ok(msg) => println!("{msg}"),
                Err(msg) => anyhow::bail!(msg),
            }
        }
        Command::Apply {
            targets,
            out_dir,
            db_name,
            dry_run,
        } => {
            let targets = parse_targets(&std::fs::read_to_string(&targets)?)?;
            // マイグレーションは当てない（`PADDOCK_AUTO_MIGRATE` に従うと、DB 名の照合の前に接続先へ DDL が流れうる）
            let pool = pool::connect_checked(&config.paddock_db_url, false)
                .await
                .context("connect Postgres")?;
            // pool を閉じてから終える（#717: 閉じずに終えた接続がサーバ側に残ることがある）
            let report = pool::close_after(&pool, async {
                let repo = PostgresRepository::new(pool.clone());
                apply::check_db_name(&repo.current_database().await?, &db_name)?;
                anyhow::Ok(apply::run(&repo, &targets, &out_dir, dry_run).await)
            })
            .await?;
            let mut total = 0usize;
            for r in &report.races {
                let nums = r.inserted.as_ref().unwrap_or(&r.planned);
                total += nums.len();
                println!(
                    "{}: 既存 {} 行・結果ページの出走馬 {} 頭・{} {} 行 {:?}",
                    r.race_id,
                    r.existing,
                    r.page_runners,
                    if dry_run { "INSERT 予定" } else { "INSERT" },
                    nums.len(),
                    nums
                );
            }
            for (race_id, why) in &report.failures {
                println!("失敗: {race_id}: {why}");
            }
            println!(
                "{}合計 {total} 行（処理 {} レース・失敗 {} 件）{}",
                if dry_run { "[dry-run] " } else { "" },
                report.races.len(),
                report.failures.len(),
                if dry_run { "・書き込みなし" } else { "" }
            );
            anyhow::ensure!(
                report.failures.is_empty(),
                "失敗したレースがあります（上の「失敗:」の行を確認してください）"
            );
        }
    }
    Ok(())
}
