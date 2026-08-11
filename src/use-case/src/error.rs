use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("not found: {0}")]
    NotFound(String),
    /// 既に存在するリソースの再作成（例: 同一開催日のセッション二重作成）。HTTP 409 に対応する。
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("internal error: {0}")]
    Internal(String),
    /// An outbound fetch (e.g. a JRA PDF GET) failed for a non-timeout reason:
    /// connection refused, 5xx, malformed response, body read error, etc. Kept
    /// distinct from `Internal` so monitoring can tell network failures from
    /// genuine internal bugs.
    #[error("fetch error: {0}")]
    Fetch(String),
    /// An outbound fetch exceeded its configured timeout. Separated from
    /// `Fetch` so a stalled-network signal stands out in logs/metrics.
    #[error("fetch timed out: {0}")]
    Timeout(String),
    /// 仕様として取り込み対象外のリソース（例: 障害レースの出馬表）。取得元は正常で、
    /// paddock 側が意図的に扱わないケース。実障害（`Internal` / `Fetch`）と分けることで、
    /// 呼び出し側が「設計どおりのスキップ」を失敗として数えずに済む（#586）。
    #[error("unsupported: {0}")]
    Unsupported(String),
}

pub type Result<A> = std::result::Result<A, Error>;

impl From<paddock_domain::Error> for Error {
    fn from(value: paddock_domain::Error) -> Self {
        Error::InvalidArgument(value.to_string())
    }
}
