use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("netkeiba fetch failed: {0}")]
    Fetch(String),
    #[error("netkeiba parse failed: {0}")]
    Parse(String),
}

pub type Result<A> = std::result::Result<A, Error>;

impl From<Error> for paddock_use_case::Error {
    fn from(value: Error) -> Self {
        match value {
            // ネットワーク/HTTP 失敗（接続リセット・タイムアウト・5xx 等）は `Fetch` に保つ。
            // ingest 側がこれを transient と判定し degraded 分岐へ回せるようにする（#288）。
            // 文言は維持（`Error::Fetch` の Display が "netkeiba fetch failed: ..." を前置）。
            Error::Fetch(_) => paddock_use_case::Error::Fetch(value.to_string()),
            // パース失敗（未発売 status=yoso 等の想定外 status を含む）は内部扱い。
            // ingest は best-effort（card+近走を巻き添えにせず継続）に倒す。
            Error::Parse(_) => paddock_use_case::Error::Internal(value.to_string()),
        }
    }
}
