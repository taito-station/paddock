use paddock_use_case::{FetchDownload, FetchFailure, FetchRecord, FetchStatus};
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

/// Stage2: ingest 成功を記録する（status='ingested'）。`failed` 行から成功へ遷移した場合に
/// `http_status` を NULL へ戻す（再試行が成功した状態を正しく表す）。
pub async fn record(pool: &PgPool, record: &FetchRecord) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO fetch_history
            (source_key, url, races_saved, horses_saved, fetched_at, status, http_status, last_attempt_at)
        VALUES ($1, $2, $3, $4, $5, 'ingested', NULL, $5)
        ON CONFLICT(source_key) DO UPDATE SET
            url = excluded.url,
            races_saved = excluded.races_saved,
            horses_saved = excluded.horses_saved,
            fetched_at = excluded.fetched_at,
            status = 'ingested',
            http_status = NULL,
            last_attempt_at = excluded.last_attempt_at
        "#,
    )
    .bind(&record.source_key)
    .bind(&record.url)
    .bind(record.races_saved as i64)
    .bind(record.horses_saved as i64)
    .bind(record.fetched_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Stage1: ダウンロード済み（inbox 保存済み・未 ingest）を記録する（status='downloaded'）。
/// 件数は ingest 時に確定するため 0 で入れる。`--force` での再ダウンロードは ingest 済みを
/// downloaded へ戻す（inbox に新しい PDF が再 ingest 待ちで置かれた状態を表す）。このとき
/// 旧 ingest 時の races_saved/horses_saved も 0 へ戻し、「未 ingest=件数未確定」を保つ。
/// `failed` 行からの再ダウンロードでは `http_status` を NULL へ戻す。
///
/// 注意: `--force --download-only` で ingested を downloaded へ戻したら、続けて `ingest` で
/// 消化すること。ingest し忘れると `fetch_history_contains`（ingested 判定）が一時的に false
/// となり、当該開催が「未取得」に見える窓ができる（再 ingest で解消）。`races` 等の既存行は
/// 残るため再 ingest は冪等。
pub async fn record_download(pool: &PgPool, download: &FetchDownload) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO fetch_history
            (source_key, url, races_saved, horses_saved, fetched_at, status, http_status, last_attempt_at)
        VALUES ($1, $2, 0, 0, $3, 'downloaded', NULL, $3)
        ON CONFLICT(source_key) DO UPDATE SET
            url = excluded.url,
            races_saved = 0,
            horses_saved = 0,
            fetched_at = excluded.fetched_at,
            status = 'downloaded',
            http_status = NULL,
            last_attempt_at = excluded.last_attempt_at
        "#,
    )
    .bind(&download.source_key)
    .bind(&download.url)
    .bind(download.downloaded_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// 取得失敗（403/404）を `failed` として記録する（#170 / ADR0024 論点1）。新規は `attempts=1`、
/// 既存行への再失敗は `attempts` を +1 する（再試行/バックオフ判断の入力）。`fetched_at` は
/// 成功時のみ入る値なので失敗では触らない（新規は NULL）。除外フラグではなく再試行の入力。
pub async fn record_failure(pool: &PgPool, failure: &FetchFailure) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO fetch_history
            (source_key, url, races_saved, horses_saved, fetched_at, status, http_status, attempts, last_attempt_at)
        VALUES ($1, $2, 0, 0, NULL, 'failed', $3, 1, $4)
        ON CONFLICT(source_key) DO UPDATE SET
            url = excluded.url,
            status = 'failed',
            http_status = excluded.http_status,
            attempts = fetch_history.attempts + 1,
            last_attempt_at = excluded.last_attempt_at
        "#,
    )
    .bind(&failure.source_key)
    .bind(&failure.url)
    .bind(failure.http_status as i32)
    .bind(failure.attempted_at)
    .execute(pool)
    .await?;
    Ok(())
}
