use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use crate::error::Result;

pub use sqlx::SqlitePool;

/// 同一クローン内の並行プロセス（predict 中の analyze 等）がロック競合したとき、即時
/// `database is locked` で失敗させず最大この時間リトライ待ちさせる(#120)。
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn connect(database_url: &str) -> Result<SqlitePool> {
    let opts: SqliteConnectOptions = database_url.parse()?;
    let opts = opts
        .create_if_missing(true)
        .foreign_keys(true)
        .busy_timeout(BUSY_TIMEOUT)
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
