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

    /// Fetch a JRA meeting-day result PDF and ingest it.
    ///
    /// The PDF is parsed in memory and never written to disk; only a
    /// fetch-history row is kept. Already-ingested meetings are skipped
    /// unless `--force` is given.
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
    #[arg(long)]
    pub venue: String,

    /// Meeting round (開催回), e.g. 3.
    #[arg(long)]
    pub round: u32,

    /// Meeting day (日次), e.g. 6.
    #[arg(long)]
    pub day: u32,

    /// Re-fetch and re-ingest even if the meeting is already in fetch history.
    #[arg(long)]
    pub force: bool,
}
