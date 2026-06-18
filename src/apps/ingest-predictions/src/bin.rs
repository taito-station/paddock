mod cli;
mod input;
mod render;

use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use paddock_config::Config;
use paddock_use_case::repository::PadPredictionRepository;
use rdb_gateway::{PostgresRepository, pool};

/// 既定の pad ルート（web-viewer と同じ iCloud Obsidian vault）。`PADDOCK_PAD_DIR` で上書き可。
const DEFAULT_PAD_DIR: &str =
    "/Users/ito-taito/Library/Mobile Documents/iCloud~md~obsidian/Documents/default/pad";

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::Cli::parse();
    let config = Config::from_env().context("load config")?;

    let pool = pool::connect(&config.paddock_db_url)
        .await
        .context("connect Postgres pool")?;
    pool::migrate(&pool).await.context("apply migrations")?;
    let repo = PostgresRepository::new(pool);

    if args.render {
        return render_all(&repo, args.pad_dir).await;
    }

    ingest(&repo, args.input, args.dry_run).await
}

async fn ingest(repo: &PostgresRepository, input: Option<PathBuf>, dry_run: bool) -> Result<()> {
    let raw = match &input {
        Some(path) => std::fs::read_to_string(path)
            .with_context(|| format!("入力 JSON を読めません: {}", path.display()))?,
        None => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("標準入力を読めません")?;
            buf
        }
    };

    let predictions = input::parse(&raw)?;

    if dry_run {
        println!("dry-run: {} 件をパース（保存しません）", predictions.len());
        for p in &predictions {
            println!(
                "  - {} {}{}R  馬{}頭 / 買い目{}点{}",
                p.date,
                p.venue.as_jp(),
                p.race_num,
                p.horses.len(),
                p.bets.len(),
                if p.result.is_some() {
                    " / 結果あり"
                } else {
                    ""
                }
            );
        }
        return Ok(());
    }

    let now = Utc::now();
    for p in &predictions {
        repo.save_pad_prediction(p, now)
            .await
            .with_context(|| format!("保存失敗: {} {}{}R", p.date, p.venue.as_jp(), p.race_num))?;
    }
    println!("ingest: {} 件を保存しました", predictions.len());
    Ok(())
}

async fn render_all(repo: &PostgresRepository, pad_dir: Option<PathBuf>) -> Result<()> {
    let pad_root = pad_dir
        .or_else(|| std::env::var("PADDOCK_PAD_DIR").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_PAD_DIR));

    let predictions = repo.list_pad_predictions().await?;
    for p in &predictions {
        let path = render::write_md(&pad_root, p)?;
        println!("  生成: {}", path.display());
    }
    println!(
        "render: {} 件の MD を {} に出力しました",
        predictions.len(),
        pad_root.display()
    );
    Ok(())
}
