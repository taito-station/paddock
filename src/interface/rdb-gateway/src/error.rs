use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("sql error: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("domain error: {0}")]
    Domain(#[from] paddock_domain::Error),
}

pub type Result<A> = std::result::Result<A, Error>;

impl From<Error> for paddock_use_case::Error {
    fn from(value: Error) -> Self {
        paddock_use_case::Error::Internal(value.to_string())
    }
}
