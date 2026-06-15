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

    /// 1 レースあたりの予算（円、軸流しポートフォリオの上限）。実上限は min(race_budget, 残高)。
    #[arg(long, default_value_t = 5000)]
    pub race_budget: u64,

    /// 中断した同日セッションを保存済みの残高から再開する。
    #[arg(long, conflicts_with_all = ["summary", "settle"])]
    pub resume: bool,

    /// 同日セッションの収支サマリと買い目明細を表示して終了する（読み取り専用）。
    #[arg(long)]
    pub summary: bool,

    /// レース確定後の事後精算。netkeiba の確定払戻で購入済み買い目の payout を自動セットし、
    /// セッションの収支・回収率を更新する（冪等。未確定レースはスキップ）。
    #[arg(long, conflicts_with = "summary")]
    pub settle: bool,
}
