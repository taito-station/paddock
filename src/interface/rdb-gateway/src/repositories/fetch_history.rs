use paddock_use_case::{FetchDownload, FetchRecord, FetchStatus};
use sqlx::PgPool;

use crate::error::Result;

/// Whether the meeting-day has been **ingested** (Stage2 完了)。ダウンロード済み・
/// 未 ingest（status='downloaded'）の行は「取得済み」とはみなさない。
pub async fn contains(pool: &PgPool, source_key: &str) -> Result<bool> {
    // Select the PK (TEXT) rather than a bare `SELECT 1`: Postgres types the
    // integer literal `1` as INT4, which fails to decode into a Rust i64 (INT8).
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT source_key FROM fetch_history WHERE source_key = $1 AND status = 'ingested' LIMIT 1",
    )
    .bind(source_key)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// 取得ライフサイクルの現在状態。履歴に無ければ `None`。
pub async fn status(pool: &PgPool, source_key: &str) -> Result<Option<FetchStatus>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT status FROM fetch_history WHERE source_key = $1 LIMIT 1")
            .bind(source_key)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|(s,)| FetchStatus::from_db_str(&s)))
}

/// Stage2: ingest 成功を記録する（status='ingested'）。
pub async fn record(pool: &PgPool, record: &FetchRecord) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO fetch_history (source_key, url, races_saved, horses_saved, fetched_at, status)
        VALUES ($1, $2, $3, $4, $5, 'ingested')
        ON CONFLICT(source_key) DO UPDATE SET
            url = excluded.url,
            races_saved = excluded.races_saved,
            horses_saved = excluded.horses_saved,
            fetched_at = excluded.fetched_at,
            status = 'ingested'
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

/// Stage1: ダウンロード済み（inbox 保存済み・未 ingest）を記録する（status='downloaded'）。
/// 件数は ingest 時に確定するため 0 で入れる。`--force` での再ダウンロードは ingest 済みを
/// downloaded へ戻す（inbox に新しい PDF が再 ingest 待ちで置かれた状態を表す）。このとき
/// 旧 ingest 時の races_saved/horses_saved も 0 へ戻し、「未 ingest=件数未確定」を保つ。
pub async fn record_download(pool: &PgPool, download: &FetchDownload) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO fetch_history (source_key, url, races_saved, horses_saved, fetched_at, status)
        VALUES ($1, $2, 0, 0, $3, 'downloaded')
        ON CONFLICT(source_key) DO UPDATE SET
            url = excluded.url,
            races_saved = 0,
            horses_saved = 0,
            fetched_at = excluded.fetched_at,
            status = 'downloaded'
        "#,
    )
    .bind(&download.source_key)
    .bind(&download.url)
    .bind(download.downloaded_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}
