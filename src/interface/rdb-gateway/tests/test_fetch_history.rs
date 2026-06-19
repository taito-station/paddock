//! #147: fetch_history の取得ライフサイクル（downloaded → ingested）を Postgres で検証する。
//! - Stage1 `record_download` は status='downloaded'。`fetch_history_contains`（ingest 済み判定）は false。
//! - Stage2 `record_fetch` は status='ingested' に遷移し、件数を確定。`contains` は true。
//! - source_key ごとに独立。

use chrono::Utc;
use paddock_use_case::repository::{
    FetchDownload, FetchFailure, FetchRecord, FetchRepository, FetchStatus,
};
use rdb_gateway::PostgresRepository;

fn download(source_key: &str) -> FetchDownload {
    FetchDownload {
        source_key: source_key.to_string(),
        url: format!("https://example/{source_key}.pdf"),
        downloaded_at: Utc::now(),
    }
}

fn failure(source_key: &str, http_status: u16) -> FetchFailure {
    FetchFailure {
        source_key: source_key.to_string(),
        url: format!("https://example/{source_key}.pdf"),
        http_status,
        attempted_at: Utc::now(),
    }
}

/// `fetch_history` の追跡カラムを直接読む（リポジトリ API は status しか公開しないため）。
async fn tracking_row(
    pool: &sqlx::PgPool,
    source_key: &str,
) -> (
    String,
    i32,
    Option<i32>,
    Option<chrono::DateTime<chrono::Utc>>,
) {
    sqlx::query_as(
        "SELECT status, attempts, http_status, last_attempt_at
         FROM fetch_history WHERE source_key = $1",
    )
    .bind(source_key)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn record(source_key: &str, races: u32, horses: u32) -> FetchRecord {
    FetchRecord {
        source_key: source_key.to_string(),
        url: format!("https://example/{source_key}.pdf"),
        races_saved: races,
        horses_saved: horses,
        fetched_at: Utc::now(),
    }
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn download_then_ingest_advances_the_lifecycle(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    let key = "2026-3-nakayama-6";

    // 未取得: 状態なし、ingest 済みでもない。
    assert_eq!(repo.fetch_status(key).await.unwrap(), None);
    assert!(!repo.fetch_history_contains(key).await.unwrap());

    // Stage1: ダウンロード済み。まだ ingest 済みではない。
    repo.record_download(&download(key)).await.unwrap();
    assert_eq!(
        repo.fetch_status(key).await.unwrap(),
        Some(FetchStatus::Downloaded)
    );
    assert!(
        !repo.fetch_history_contains(key).await.unwrap(),
        "downloaded だけでは ingest 済み扱いにしない（再 ingest 余地を残す）"
    );

    // Stage2: ingest 完了で ingested へ遷移。
    repo.record_fetch(&record(key, 12, 180)).await.unwrap();
    assert_eq!(
        repo.fetch_status(key).await.unwrap(),
        Some(FetchStatus::Ingested)
    );
    assert!(repo.fetch_history_contains(key).await.unwrap());
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn ingest_without_prior_download_is_ingested(pool: sqlx::PgPool) {
    // 一発 fetch（Stage1 を経ずに parse まで実施）は直接 ingested になる。
    let repo = PostgresRepository::new(pool);
    let key = "2026-1-tokyo-1";

    repo.record_fetch(&record(key, 1, 16)).await.unwrap();
    assert_eq!(
        repo.fetch_status(key).await.unwrap(),
        Some(FetchStatus::Ingested)
    );
    assert!(repo.fetch_history_contains(key).await.unwrap());
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn lifecycle_is_per_source_key(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    repo.record_download(&download("2026-3-nakayama-6"))
        .await
        .unwrap();
    repo.record_fetch(&record("2026-3-nakayama-7", 11, 165))
        .await
        .unwrap();

    assert_eq!(
        repo.fetch_status("2026-3-nakayama-6").await.unwrap(),
        Some(FetchStatus::Downloaded)
    );
    assert_eq!(
        repo.fetch_status("2026-3-nakayama-7").await.unwrap(),
        Some(FetchStatus::Ingested)
    );
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn check_constraint_rejects_unknown_status(pool: sqlx::PgPool) {
    // status は CHECK (status IN ('downloaded','ingested')) で守られ、不正値は DB が弾く。
    let result = sqlx::query(
        "INSERT INTO fetch_history (source_key, url, races_saved, horses_saved, fetched_at, status)
         VALUES ('2026-3-nakayama-6', 'https://example/x.pdf', 0, 0, '2026-01-01T00:00:00+00:00', 'bogus')",
    )
    .execute(&pool)
    .await;
    assert!(
        result.is_err(),
        "CHECK 制約が不正な status='bogus' の INSERT を弾くはず"
    );
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn record_failure_creates_failed_row_with_status_and_attempts(pool: sqlx::PgPool) {
    // #170: 取得失敗を failed として記録する。status/http_status/attempts/last_attempt_at を持つ。
    let repo = PostgresRepository::new(pool.clone());
    let key = "2026-2-tokyo-12";

    repo.record_failure(&failure(key, 403)).await.unwrap();

    assert_eq!(
        repo.fetch_status(key).await.unwrap(),
        Some(FetchStatus::Failed),
        "failed として記録される"
    );
    // failed 行は ingest 済みではない（再取得候補のまま）。
    assert!(!repo.fetch_history_contains(key).await.unwrap());

    let (status, attempts, http_status, last_attempt_at) = tracking_row(&pool, key).await;
    assert_eq!(status, "failed");
    assert_eq!(attempts, 1, "初回失敗は attempts=1");
    assert_eq!(http_status, Some(403));
    assert!(last_attempt_at.is_some(), "last_attempt_at が記録される");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn repeated_record_failure_increments_attempts(pool: sqlx::PgPool) {
    // 再試行のたびに attempts が増える（バックオフ/再試行判断の入力）。
    let repo = PostgresRepository::new(pool.clone());
    let key = "2026-2-tokyo-12";

    repo.record_failure(&failure(key, 404)).await.unwrap();
    repo.record_failure(&failure(key, 403)).await.unwrap();

    let (_, attempts, http_status, _) = tracking_row(&pool, key).await;
    assert_eq!(attempts, 2, "2 回目の失敗で attempts=2");
    assert_eq!(http_status, Some(403), "最新の http_status へ更新される");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn failure_then_download_transitions_and_clears_http_status(pool: sqlx::PgPool) {
    // failed → 再取得成功（downloaded）で status が遷移し http_status は NULL へ戻る。
    let repo = PostgresRepository::new(pool.clone());
    let key = "2026-2-tokyo-12";

    repo.record_failure(&failure(key, 403)).await.unwrap();
    repo.record_download(&download(key)).await.unwrap();

    assert_eq!(
        repo.fetch_status(key).await.unwrap(),
        Some(FetchStatus::Downloaded)
    );
    let (status, attempts, http_status, _) = tracking_row(&pool, key).await;
    assert_eq!(status, "downloaded");
    assert_eq!(http_status, None, "成功遷移で http_status はクリアされる");
    assert_eq!(
        attempts, 0,
        "成功遷移で attempts（失敗の連なり）はリセットされる"
    );
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn record_failure_over_ingested_clears_success_metadata(pool: sqlx::PgPool) {
    // --force で既存 ingested 行（races_saved>0, fetched_at 有り）を境界 403 で踏むケース。
    // failed 行は「成功メタを持たない」を不変条件とするため、races_saved/horses_saved/fetched_at
    // が 0/0/NULL へ戻ることを確認する（旧成功スナップショットを失敗行に残さない）。
    let repo = PostgresRepository::new(pool.clone());
    let key = "2026-2-tokyo-12";

    repo.record_fetch(&record(key, 12, 180)).await.unwrap();
    repo.record_failure(&failure(key, 403)).await.unwrap();

    assert_eq!(
        repo.fetch_status(key).await.unwrap(),
        Some(FetchStatus::Failed)
    );
    let row: (i64, i64, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT races_saved, horses_saved, fetched_at FROM fetch_history WHERE source_key = $1",
    )
    .bind(key)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.0, 0, "failed 行は races_saved を残さない");
    assert_eq!(row.1, 0, "failed 行は horses_saved を残さない");
    assert_eq!(row.2, None, "failed 行は fetched_at を NULL にする");
}
