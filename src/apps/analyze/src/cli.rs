use clap::{Parser, Subcommand};

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
    /// Note: win <= place <= show monotonicity is NOT guaranteed (independent normalization).
    Predict {
        /// Race ID (例: 2026060412R02).
        race_id: String,
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
    },
}
