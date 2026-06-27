use chrono::NaiveDate;
use clap::Parser;

/// 発走直前 EV 監視（#257）。指定開催日の発走前レースを定期的にスキャンし、
/// 発走直前のフレッシュなオッズで EV/ROI を再計算して ROI≥ゲートをアラートする。
/// predict のセッション記録（predict_sessions / predict_bets＝買い目）には一切書き込まない。
/// 監視のためオッズスナップショット（race_odds）の再取得・保存は行う。
#[derive(Debug, Parser)]
#[command(
    name = "paddock-predict-watch",
    about = "発走直前のフレッシュなオッズで EV/ROI を再計算し ROI≥ゲートを通知する（セッション記録は不変）",
    version
)]
pub struct Cli {
    /// 対象開催日（YYYY-MM-DD）。ライブ監視前提のため通常は当日を指定する。
    #[arg(long)]
    pub date: NaiveDate,

    /// 先読み窓（分）。発走まで残りこの時間以内のレースだけを対象にオッズを再取得する。
    #[arg(long, default_value_t = 40)]
    pub window: u64,

    /// スイープ間隔（分）。1 巡したらこの時間だけ待って再スキャンする。
    #[arg(long, default_value_t = 5)]
    pub interval: u64,

    /// アラート閾値の ROI（期待回収率）。これ以上のレースを 🟢 として通知する（既定 1.0 = 100%）。
    #[arg(long, default_value_t = 1.0)]
    pub roi_gate: f64,

    /// 1 レースあたりの予算（円）。買い目（軸流しポートフォリオ）の組成上限。
    #[arg(long, default_value_t = 5000)]
    pub race_budget: u64,

    /// 市場単勝ブレンドのモデル重み α（blended = α·model + (1-α)·market）。未指定なら本番既定 α=0.2。
    /// 値を渡せば上書きするが、ブレンド無効化（predict の α=None 経路）は CLI からは選べない。
    #[arg(long)]
    pub blend_alpha: Option<f64>,

    /// オッズ再取得時の 1 リクエストごとの待機（ミリ秒）。JRA への礼節のため間隔を空ける。
    #[arg(long, default_value_t = 3000)]
    pub scrape_delay: u64,

    /// 1 スイープだけ実行して終了する（テスト・cron 用）。未指定なら全レース発走まで監視を続ける。
    #[arg(long)]
    pub once: bool,
}
