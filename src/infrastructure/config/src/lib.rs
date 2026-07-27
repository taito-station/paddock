use serde::Deserialize;
use thiserror::Error;
use tracing_subscriber::{EnvFilter, fmt};

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
    /// 起動時に DB マイグレーションを自動適用するか（#470）。既定 `false`＝自動適用しない。
    /// 共有 golden DB を複数 worktree/バイナリが叩くため、既定では起動時に DDL を発行せず
    /// read-only 整合チェックのみ行い、明示適用（`paddock-analyze migrate`）に一本化する。
    /// prod（compose の `PADDOCK_AUTO_MIGRATE=true`）だけ従来どおり起動時 auto-migrate を有効化する。
    #[serde(default = "default_auto_migrate")]
    pub paddock_auto_migrate: bool,
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
    // パース結果自体は得られるためノイズでしかなく、html5ever ターゲットに
    // 限定して off にし、他の有用な WARN は残す。
    // （selectors は実測でノイズを出さなかったため抑止対象から外している）
    "info,html5ever=off".to_string()
}

fn default_server_addr() -> String {
    "127.0.0.1:8080".to_string()
}

/// 起動時 auto-migrate の既定（#470）。既定は `false`＝起動時に自動適用しない。
fn default_auto_migrate() -> bool {
    false
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let _ = dotenvy::dotenv();
        envy::from_env::<Config>().map_err(|e| Error::Env(e.to_string()))
    }

    /// tracing subscriber を `paddock_log` フィルタで初期化する（#410）。全 app の build_app が
    /// 同一の `fmt().with_env_filter(...).try_init()` を重複していたのを集約する。フィルタが不正な
    /// 文字列（typo 等）なら `info` にフォールバックする（#238 の html5ever 抑止が黙って無効化されるのを
    /// 防ぐ回帰は default_log_filter_is_valid_env_filter で担保）。`try_init` のため二重初期化は無害に無視。
    pub fn init_tracing(&self) {
        let _ = fmt()
            .with_env_filter(
                EnvFilter::try_new(self.paddock_log.clone())
                    .unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .try_init();
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
    }

    /// 起動時 auto-migrate の既定は false（#470）。共有 DB へ起動時に無条件 DDL を打たない。
    #[test]
    fn default_auto_migrate_is_false() {
        assert!(!default_auto_migrate());
    }
}
