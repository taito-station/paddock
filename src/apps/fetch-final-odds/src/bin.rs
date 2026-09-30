use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use fetch_final_odds::fetch::run;
use fetch_final_odds::targets::parse_targets;
use fetch_final_odds::{
    MAX_NON_RESULT_STREAK, MIN_INTERVAL_MS, emit, ensure_outside_repository, exit_outcome,
    validate_interval,
};
use netkeiba_scraper::UreqNetkeibaScraper;

/// 確定オッズ（馬連・ワイド・三連複）を netkeiba から遡って取得し、生の応答と TSV を保存する（#721）。
/// DB には書かない。
#[derive(Parser, Debug)]
#[command(name = "paddock-fetch-final-odds")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// 対象一覧のレースの確定オッズを取得して `<out>/raw/` に保存する（再開可能）。
    Fetch {
        /// 対象レース一覧（1 行 1 つの paddock race_id。先頭行 `race_id` はヘッダ）。
        #[arg(long)]
        races: PathBuf,
        /// 出力先ディレクトリ。リポジトリの中は拒否する（取得データを公開リポジトリに混ぜない）。
        #[arg(long)]
        out_dir: PathBuf,
        /// リクエスト間隔（ミリ秒）。3334 未満（max-rps 0.3 を超える）は受け付けない。
        #[arg(long, default_value_t = MIN_INTERVAL_MS)]
        interval_ms: u64,
    },
    /// `<out>/raw/` の応答から `final_odds.tsv` と `index.tsv` を作る（ネットワーク不使用）。
    Emit {
        #[arg(long)]
        out_dir: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    // ログは他の app と同じく paddock-config の init_tracing（PADDOCK_LOG・既定 info,html5ever=off）。DB には接続しない。
    paddock_config::Config::from_env()?.init_tracing();
    match Cli::parse().command {
        Command::Fetch {
            races,
            out_dir,
            interval_ms,
        } => {
            validate_interval(interval_ms)?;
            ensure_outside_repository(&out_dir)?;
            let targets = parse_targets(&std::fs::read_to_string(&races)?)?;
            println!(
                "対象 {} レース × 3 券種・間隔 {interval_ms}ms → {}",
                targets.len(),
                out_dir.display()
            );
            let scraper = UreqNetkeibaScraper::with_delay(Duration::from_millis(interval_ms));
            let (summary, stop) = run(
                &targets,
                &scraper,
                &out_dir,
                MAX_NON_RESULT_STREAK,
                |done, total, s| {
                    if s.fetched % 60 == 0 {
                        println!(
                            "[{done}/{total}] 取得 {} / スキップ {} / 非確定 {}",
                            s.fetched,
                            s.skipped,
                            s.non_result.len()
                        );
                    }
                },
            )?;
            for (race_id, t, status) in &summary.non_result {
                println!("非確定: {race_id} type={t} status={status}");
            }
            println!(
                "取得 {} / スキップ {} / 非確定 {}",
                summary.fetched,
                summary.skipped,
                summary.non_result.len()
            );
            match exit_outcome(&summary, &stop) {
                Ok(msg) => println!("{msg}"),
                Err(msg) => anyhow::bail!(msg),
            }
        }
        Command::Emit { out_dir } => {
            let st = emit::emit_dir(&out_dir)?;
            println!(
                "{} ファイル → {} 行（{}）",
                st.files,
                st.rows,
                out_dir.join("final_odds.tsv").display()
            );
            if st.empty_results > 0 {
                println!(
                    "警告: 確定の応答なのに組合せが 0 件のファイルが {} 件あります（中止など。index.tsv の entries=0）",
                    st.empty_results
                );
            }
        }
    }
    Ok(())
}
