mod cli;
mod input;
mod setup;

use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use paddock_use_case::{Interactor, NoopFetcher, NoopParser};
use rdb_gateway::PostgresRepository;

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::Cli::parse();
    let app = setup::build_app().await?;
    ingest(&app.interactor, args.input, args.dry_run).await
}

async fn ingest(
    interactor: &Interactor<PostgresRepository, NoopParser, NoopFetcher>,
    input: Option<PathBuf>,
    dry_run: bool,
) -> Result<()> {
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
        interactor
            .ingest_pad_prediction(p, now)
            .await
            .with_context(|| format!("保存失敗: {} {}{}R", p.date, p.venue.as_jp(), p.race_num))?;
    }
    println!("ingest: {} 件を保存しました", predictions.len());
    Ok(())
}
