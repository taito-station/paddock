use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "paddock-parse-pdf",
    about = "Parse JRA race-result PDFs and store the data into SQLite",
    version,
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Default (no subcommand): ingest the given PDF sources.
    #[command(flatten)]
    pub ingest: IngestArgs,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Ingest PDFs from local paths or http(s) URLs (the default action).
    Ingest(IngestArgs),

    /// Fetch JRA meeting-day result PDF(s) and ingest them.
    ///
    /// Specify a single day with `--year --venue --round --day`, or widen the
    /// range by omitting trailing fields: drop `--day` for the whole round,
    /// `--round` for the whole venue, `--venue` for the entire year.
    ///
    /// PDFs are parsed in memory and never written to disk; only fetch-history
    /// rows are kept. Already-ingested meetings are skipped unless `--force`.
    Fetch(FetchArgs),
}

#[derive(Debug, Args)]
pub struct IngestArgs {
    /// PDF sources: local paths or http(s) URLs (one or more).
    #[arg(required = true)]
    pub sources: Vec<String>,

    /// Maximum number of PDFs processed concurrently (default: number of CPU cores).
    #[arg(short = 'j', long)]
    pub parallel: Option<usize>,
}

#[derive(Debug, Args)]
pub struct FetchArgs {
    /// Meeting year, e.g. 2026.
    #[arg(long)]
    pub year: i32,

    /// Venue, Japanese name or slug (e.g. "中山" or "nakayama").
    /// Omit to fetch every venue in the year.
    #[arg(long)]
    pub venue: Option<String>,

    /// Meeting round (開催回), e.g. 3. Omit to fetch every round of the venue.
    #[arg(long)]
    pub round: Option<u32>,

    /// Meeting day (日次), e.g. 6. Omit to fetch every day of the round.
    #[arg(long)]
    pub day: Option<u32>,

    /// Seconds to wait between JRA requests during a range fetch (default 1.0).
    #[arg(long, default_value_t = 1.0)]
    pub interval: f64,

    /// Re-fetch and re-ingest even if the meeting is already in fetch history.
    #[arg(long)]
    pub force: bool,
}
