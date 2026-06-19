//! #147: fetch_history の取得ライフサイクル（downloaded → ingested）を Postgres で検証する。
//! - Stage1 `record_download` は status='downloaded'。`fetch_history_contains`（ingest 済み判定）は false。
//! - Stage2 `record_fetch` は status='ingested' に遷移し、件数を確定。`contains` は true。
//! - source_key ごとに独立。

use chrono::Utc;
use paddock_use_case::repository::{FetchDownload, FetchRecord, FetchRepository, FetchStatus};
use rdb_gateway::PostgresRepository;

fn download(source_key: &str) -> FetchDownload {
    FetchDownload {
        source_key: source_key.to_string(),
        url: format!("https://example/{source_key}.pdf"),
        downloaded_at: Utc::now(),
    }
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
