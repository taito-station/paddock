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
    /// api-server の netkeiba 取得のリクエスト間隔（ミリ秒・#721）。未設定なら scraper の既定（1 秒）。
    /// 過去日の着順を `results:refresh` でまとめて取り込む専用のサーバで、バルクの礼儀ペーシング（3,334ms 以上
    /// ＝ max-rps 0.3 相当）へ**広げる**ためだけに使う。odds:refresh（盤面のライブ取得）にも効くので `.env` には
    /// 常設しない。既定より短くはできない（[`Config::netkeiba_interval`] が拒否する）。
    #[serde(default)]
    pub paddock_netkeiba_interval_ms: Option<u64>,
}

/// netkeiba 取得間隔の下限（ミリ秒）。scraper の既定と同じで、これより速くは叩かせない。
pub const MIN_NETKEIBA_INTERVAL_MS: u64 = 1000;

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

    /// netkeiba 取得間隔の指定（#721）。未設定は `None`（scraper の既定）。既定の 1 秒より短い指定はエラー
    /// （設定ひとつで礼儀ペーシングを外せないようにする。単位の取り違え `=3`〈3ms〉も止める）。
    pub fn netkeiba_interval(&self) -> Result<Option<std::time::Duration>> {
        match self.paddock_netkeiba_interval_ms {
            None => Ok(None),
            Some(ms) if ms < MIN_NETKEIBA_INTERVAL_MS => Err(Error::Env(format!(
                "PADDOCK_NETKEIBA_INTERVAL_MS は {MIN_NETKEIBA_INTERVAL_MS} 以上にしてください（指定値 {ms}）"
            ))),
            Some(ms) => Ok(Some(std::time::Duration::from_millis(ms))),
        }
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

    /// netkeiba の取得間隔は既定では指定しない（scraper の既定を使う・#721）。
    #[test]
    fn netkeiba_interval_is_unset_by_default() {
        let config: Config = envy::from_iter(Vec::<(String, String)>::new()).unwrap();
        assert_eq!(config.paddock_netkeiba_interval_ms, None);
        let set: Config = envy::from_iter(vec![(
            "PADDOCK_NETKEIBA_INTERVAL_MS".to_string(),
            "3000".to_string(),
        )])
        .unwrap();
        assert_eq!(set.paddock_netkeiba_interval_ms, Some(3000));
    }

    /// 既定（1 秒）より短い間隔は拒否し、未設定は None（scraper の既定）。
    #[test]
    fn netkeiba_interval_rejects_values_faster_than_the_default() {
        let with = |ms: Option<u64>| Config {
            paddock_netkeiba_interval_ms: ms,
            ..envy::from_iter(Vec::<(String, String)>::new()).unwrap()
        };
        assert_eq!(with(None).netkeiba_interval().unwrap(), None);
        assert!(with(Some(999)).netkeiba_interval().is_err());
        assert!(with(Some(3)).netkeiba_interval().is_err());
        assert_eq!(
            with(Some(1000)).netkeiba_interval().unwrap(),
            Some(std::time::Duration::from_millis(1000))
        );
        assert_eq!(
            with(Some(3500)).netkeiba_interval().unwrap(),
            Some(std::time::Duration::from_millis(3500))
        );
    }

    /// 起動時 auto-migrate の既定は false（#470）。共有 DB へ起動時に無条件 DDL を打たない。
    #[test]
    fn default_auto_migrate_is_false() {
        assert!(!default_auto_migrate());
    }
}
