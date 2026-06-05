use chrono::NaiveDate;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "paddock-predict",
    about = "1 日分のレースを対話的に予想し、買い目と収支を記録する",
    version
)]
pub struct Cli {
    /// 対象開催日（YYYY-MM-DD、例: 2026-06-01）。
    #[arg(long)]
    pub date: NaiveDate,

    /// 初期予算（円、例: 10000）。
    #[arg(long)]
    pub budget: u64,
}
