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
    /// REST API サーバ（api-server, #33）の bind アドレス（`host:port`）。
    #[serde(default = "default_server_addr")]
    pub paddock_server_addr: String,
}

fn default_db_url() -> String {
    "postgres://paddock:paddock@localhost:5432/paddock".to_string()
}

fn default_pdfs_dir() -> String {
    "pdfs".to_string()
}

fn default_log_filter() -> String {
    // netkeiba の HTML は table 周辺が不正構造で、scraper(html5ever) が
    // foster parenting 経路の WARN を 1 レースあたり数千行出す（#238）。
    // パース結果自体は得られるためノイズでしかなく、html5ever / selectors
    // ターゲットに限定して off にし、他の有用な WARN は残す。
    "info,html5ever=off,selectors=off".to_string()
}

fn default_server_addr() -> String {
    "127.0.0.1:8080".to_string()
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let _ = dotenvy::dotenv();
        envy::from_env::<Config>().map_err(|e| Error::Env(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::EnvFilter;

    /// 既定フィルタが EnvFilter として正しくパースできること。
    /// typo があると setup 側が黙って `info` にフォールバックし、
    /// html5ever の WARN 抑止（#238）が効かなくなるため回帰として担保する。
    #[test]
    fn default_log_filter_is_valid_env_filter() {
        EnvFilter::try_new(default_log_filter()).expect("default filter must parse");
    }

    /// netkeiba スクレイプ時の html5ever ノイズを抑止する指定を含むこと（#238）。
    #[test]
    fn default_log_filter_suppresses_html5ever() {
        let filter = default_log_filter();
        assert!(filter.contains("html5ever=off"), "got: {filter}");
        assert!(filter.contains("selectors=off"), "got: {filter}");
    }
}
