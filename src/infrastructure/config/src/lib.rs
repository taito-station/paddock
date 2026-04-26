use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("env load failed: {0}")]
    Env(String),
}

pub type Result<A> = std::result::Result<A, Error>;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_db_url")]
    pub paddock_db_url: String,
    #[serde(default = "default_pdfs_dir")]
    pub paddock_pdfs_dir: String,
    #[serde(default = "default_log_filter")]
    pub paddock_log: String,
    #[serde(default = "default_parser")]
    pub paddock_parser: String,
}

fn default_db_url() -> String {
    "sqlite://data/paddock.db?mode=rwc".to_string()
}

fn default_pdfs_dir() -> String {
    "pdfs".to_string()
}

fn default_log_filter() -> String {
    "info".to_string()
}

fn default_parser() -> String {
    "mutool".to_string()
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let _ = dotenvy::dotenv();
        envy::from_env::<Config>().map_err(|e| Error::Env(e.to_string()))
    }
}
