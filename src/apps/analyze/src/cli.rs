use clap::{Parser, Subcommand};
use paddock_domain::TrackCondition;

#[derive(Debug, Parser)]
#[command(
    name = "paddock-analyze",
    about = "Query JRA race statistics aggregated from parsed PDFs",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Stats for a single horse (overall, by surface, distance band, gate group, track condition).
    Horse {
        /// Horse name (Japanese, exact match).
        name: String,
    },
    /// Gate-position win/place rate at a specific course/distance/surface.
    Course {
        /// Venue name (例: 中山, 阪神, 東京).
        venue: String,
        /// Distance in meters (例: 2000).
        distance: u32,
        /// Surface: turf or dirt.
        surface: String,
    },
    /// Stats for a jockey (overall, by surface, by gate group).
    Jockey {
        /// Jockey name (Japanese, exact match).
        name: String,
    },
    /// Predict win/place/show probabilities for each horse in a race.
    /// win <= place <= show monotonicity is guaranteed; place/show are top-2 / top-3 probabilities
    /// (normalized to sum 2.0 / 3.0 across the field, then monotonized). See ADR 0007.
    Predict {
        /// Race ID (例: 2026060412R02).
        race_id: String,
        /// 市場オッズ(単勝)ブレンドのモデル重み α [0,1]。未指定でモデルのみ、
        /// 指定すると最新オッズスナップショット(時刻制約なし)の implied 確率と (1-α) で
        /// ブレンドする（#72）。
        #[arg(long)]
        blend_alpha: Option<f64>,
        /// 当日の馬場状態（良/稍重/重/不良。稍/不 の略記も可）。指定すると各馬の
        /// 馬場状態別成績を factor に加える（#73）。出馬表 PDF に馬場状態は無いため
        /// 手で渡す。未指定なら馬場項なし。
        #[arg(long, value_parser = parse_track_condition)]
        track_condition: Option<TrackCondition>,
    },
    /// Backtest the prediction logic over finished races in a date range.
    /// Reproduces probability estimation with as-of stats (no leakage) and reports
    /// hit rate / expected payout rate / Brier / LogLoss.
    Backtest {
        /// 開始日 YYYY-MM-DD（含む）。
        #[arg(long)]
        from: String,
        /// 終了日 YYYY-MM-DD（含む）。
        #[arg(long)]
        to: String,
        /// 市場オッズ(単勝)ブレンドのモデル重み α [0,1]。未指定でモデルのみ、
        /// 指定すると当時オッズの implied 確率と (1-α) でブレンドする（#72）。
        #[arg(long)]
        blend_alpha: Option<f64>,
    },
}

/// clap 用: 馬場状態のパース。引数解析時に検証し、不正値は usage エラーとして報告する。
fn parse_track_condition(s: &str) -> Result<TrackCondition, String> {
    TrackCondition::try_from(s).map_err(|e| e.to_string())
}
