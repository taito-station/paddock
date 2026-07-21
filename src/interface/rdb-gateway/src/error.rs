use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("sql error: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("domain error: {0}")]
    Domain(#[from] paddock_domain::Error),
    /// 永続化された値（日時・日付文字列など）が想定形式でないとき。
    #[error("data error: {0}")]
    Data(String),
    /// トランザクション内ガード（FOR UPDATE 下）で対象が見つからないとき（例: セッション未作成）。
    /// use-case の `NotFound`（HTTP 404）にマップする。
    #[error("not found: {0}")]
    NotFound(String),
    /// トランザクション内ガード（FOR UPDATE 下）でリソースの二重記録を検出したとき（例: 当該レースの
    /// outcome が既に記録済み）。use-case の `Conflict`（HTTP 409）にマップする。
    #[error("conflict: {0}")]
    Conflict(String),
    /// トランザクション内ガード（FOR UPDATE 下）で引数が不正なとき（例: 賭け金合計が残高を超過）。
    /// use-case の `InvalidArgument`（HTTP 400）にマップする。
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

pub type Result<A> = std::result::Result<A, Error>;

impl From<Error> for paddock_use_case::Error {
    fn from(value: Error) -> Self {
        match value {
            Error::NotFound(msg) => paddock_use_case::Error::NotFound(msg),
            Error::Conflict(msg) => paddock_use_case::Error::Conflict(msg),
            Error::InvalidArgument(msg) => paddock_use_case::Error::InvalidArgument(msg),
            other => paddock_use_case::Error::Internal(other.to_string()),
        }
    }
}
