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

    /// 初期予算（円、例: 10000）。新規セッションの開始時のみ必須。
    /// `--resume` / `--summary` では保存済みセッションの値を使うため不要。
    #[arg(long)]
    pub budget: Option<u64>,

    /// 中断した同日セッションを保存済みの残高から再開する。
    #[arg(long, conflicts_with = "summary")]
    pub resume: bool,

    /// 同日セッションの収支サマリと買い目明細を表示して終了する（読み取り専用）。
    #[arg(long)]
    pub summary: bool,
}
