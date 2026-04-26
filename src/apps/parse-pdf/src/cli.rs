use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "paddock-parse-pdf",
    about = "Parse a JRA race-result PDF and store the data into SQLite",
    version
)]
pub struct Cli {
    /// PDF source: a local path or an http(s) URL.
    pub source: String,
}
