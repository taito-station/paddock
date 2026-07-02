use chrono::NaiveDate;
use clap::Parser;

/// 全レースの単複オッズ時系列を終日収集するコレクタ（モデル非依存・#odds-collect）。
///
/// 指定日の全レースを間隔スイープし、**未発走レースの単複オッズだけ**（type=1・1 GET）を
/// 再取得して `race_odds_snapshots` に append する。発走済みは順次対象外、全レース発走で自動終了。
/// predict/EV/買い目は一切計算しない（確率と収集の分離）。前提: 当日 fetch-card 済み（post_time 要）。
#[derive(Parser, Debug)]
#[command(
    name = "paddock-odds-collect",
    about = "全レースの単複オッズ時系列を終日収集する（モデル非依存）"
)]
pub struct Cli {
    /// 対象開催日（YYYY-MM-DD）。当日を指定する（発走状態は現在時刻と post_time で判定）。
    #[arg(long)]
    pub date: NaiveDate,

    /// スイープ間隔（分・最小 1 分）。既定 15。0 は連続再取得＝礼節に反するため parse 時に弾く。
    #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u64).range(1..))]
    pub interval: u64,

    /// オッズ再取得の 1 リクエストごとの待機（ms・netkeiba への礼節）。既定 2000。
    #[arg(long, default_value_t = 2000)]
    pub scrape_delay: u64,

    /// 1 スイープだけ実行して終了（cron 等から定期起動する運用向け）。
    #[arg(long)]
    pub once: bool,
}
