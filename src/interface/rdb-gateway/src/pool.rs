use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use crate::error::Result;

pub use sqlx::SqlitePool;

pub async fn connect(database_url: &str) -> Result<SqlitePool> {
    let opts: SqliteConnectOptions = database_url.parse()?;
    let opts = opts
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;
    Ok(pool)
}

pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::migrate!("../../../deployments/db/migrations")
        .run(pool)
        .await?;
    Ok(())
}
