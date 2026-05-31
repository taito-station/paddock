use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "paddock-parse-entries",
    about = "Parse JRA race-card (出馬表) PDFs and store the data into SQLite",
    version
)]
pub struct Cli {
    /// PDF sources: local paths or http(s) URLs (one or more).
    #[arg(required = true)]
    pub sources: Vec<String>,

    /// Maximum number of PDFs processed concurrently (default: number of CPU cores).
    #[arg(short = 'j', long)]
    pub parallel: Option<usize>,
}
