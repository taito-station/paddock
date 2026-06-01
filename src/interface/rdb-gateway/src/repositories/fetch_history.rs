use paddock_use_case::FetchRecord;
use sqlx::SqlitePool;

use crate::error::Result;

pub async fn contains(pool: &SqlitePool, source_key: &str) -> Result<bool> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM fetch_history WHERE source_key = $1 LIMIT 1")
            .bind(source_key)
            .fetch_optional(pool)
            .await?;
    Ok(row.is_some())
}

pub async fn record(pool: &SqlitePool, record: &FetchRecord) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO fetch_history (source_key, url, races_saved, horses_saved, fetched_at)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT(source_key) DO UPDATE SET
            url = excluded.url,
            races_saved = excluded.races_saved,
            horses_saved = excluded.horses_saved,
            fetched_at = excluded.fetched_at
        "#,
    )
    .bind(&record.source_key)
    .bind(&record.url)
    .bind(record.races_saved as i64)
    .bind(record.horses_saved as i64)
    .bind(record.fetched_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}
