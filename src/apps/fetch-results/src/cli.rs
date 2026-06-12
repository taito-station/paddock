use chrono::NaiveDate;
use clap::Parser;

/// netkeiba レース結果ページから既存 `results` を再取得し、jockey/trainer を略名表記に揃える。
///
/// PDF 由来の `results.jockey` の馬主名混入と、`results.trainer` 空・フルネーム不一致を解消し、
/// predict の entry(netkeiba 略名)↔results join を噛み合わせる。races 行は更新しない。
#[derive(Parser, Debug)]
#[command(name = "paddock-fetch-results", about = "netkeiba 結果で results を再取込")]
pub struct Cli {
    /// 対象期間の開始日 YYYY-MM-DD（含む）。
    #[arg(long, default_value = "2025-01-01")]
    pub from: NaiveDate,

    /// 対象期間の終了日 YYYY-MM-DD（含む）。
    #[arg(long, default_value = "2026-12-31")]
    pub to: NaiveDate,

    /// netkeiba へのリクエスト間ウェイト(ms)。未指定は既定 1000ms。
    #[arg(long)]
    pub interval: Option<u64>,
}
