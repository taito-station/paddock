//! `check_migration_status` / `connect_checked` の read-only 整合チェック（#470）を Postgres
//! （`#[sqlx::test]` の一時 DB）で検証する。起動時 auto-migrate を止め、明示適用へ移行したため、
//! 「未適用/未初期化を副作用なしで検出できること」「未初期化に触れてもテーブルを作らないこと（read-only 回帰）」
//! を担保する。golden DB は触らない（各テストは sqlx が用意する使い捨て一時 DB を使う）。

use rdb_gateway::pool::{self, MigrationStatus};

/// 空の migrator（適用ゼロ）。空 DB ケースを作るために使う。`Migrator::DEFAULT` は migrations が空。
static EMPTY: sqlx::migrate::Migrator = sqlx::migrate::Migrator::DEFAULT;

/// 未初期化 DB を再現する。`#[sqlx::test]` の harness は EMPTY migrator でも `_sqlx_migrations` を
/// 事前作成するため、真に「テーブル不在」の状態を作るには明示的に DROP する（throwaway な一時 DB に対する操作）。
async fn drop_migrations_table(pool: &sqlx::PgPool) {
    sqlx::query("DROP TABLE IF EXISTS _sqlx_migrations")
        .execute(pool)
        .await
        .unwrap();
}

/// 全 migration 適用済み（`migrations = ...` で sqlx が適用した直後）は UpToDate。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn all_applied_is_up_to_date(pool: sqlx::PgPool) {
    let status = pool::check_migration_status(&pool).await.unwrap();
    assert_eq!(status, MigrationStatus::UpToDate);
}

/// migration 未適用の空 DB（`_sqlx_migrations` 不在）は Uninitialized。
#[sqlx::test(migrator = "EMPTY")]
async fn empty_db_is_uninitialized(pool: sqlx::PgPool) {
    drop_migrations_table(&pool).await;
    let status = pool::check_migration_status(&pool).await.unwrap();
    assert_eq!(status, MigrationStatus::Uninitialized);
}

/// 全適用後、埋め込みに無い架空 version を `_sqlx_migrations` に INSERT すると StaleBinary。
/// `_sqlx_migrations` は (version, description, success, checksum, execution_time) が NOT NULL なので
/// ダミー値を埋める（installed_on は DEFAULT now()）。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn unknown_applied_version_is_stale_binary(pool: sqlx::PgPool) {
    let phantom: i64 = 99990101000000;
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES ($1, 'phantom future migration', true, $2, 0)",
    )
    .bind(phantom)
    .bind(vec![0u8; 48])
    .execute(&pool)
    .await
    .unwrap();

    let status = pool::check_migration_status(&pool).await.unwrap();
    assert_eq!(status, MigrationStatus::StaleBinary(vec![phantom]));
}

/// 空 DB に `migrate` を適用すると、再チェックで UpToDate に落ちる（適用の往復）。
#[sqlx::test(migrator = "EMPTY")]
async fn migrate_then_up_to_date(pool: sqlx::PgPool) {
    drop_migrations_table(&pool).await;
    assert_eq!(
        pool::check_migration_status(&pool).await.unwrap(),
        MigrationStatus::Uninitialized,
        "適用前は未初期化"
    );
    pool::migrate(&pool).await.unwrap();
    assert_eq!(
        pool::check_migration_status(&pool).await.unwrap(),
        MigrationStatus::UpToDate,
        "適用後は最新"
    );
}

/// read-only 回帰: 空 DB に対する `check_migration_status` は DDL を発行せず、`_sqlx_migrations` を
/// 作らない。connect_checked(false) は Uninitialized で Err になる設計だが、その副作用として
/// テーブルを作ってしまわないことを担保する（起動時 auto-migrate 全廃の肝）。
#[sqlx::test(migrator = "EMPTY")]
async fn check_is_read_only_and_creates_no_table(pool: sqlx::PgPool) {
    drop_migrations_table(&pool).await;
    // read-only チェックは Uninitialized を返し、副作用を残さない。
    let status = pool::check_migration_status(&pool).await.unwrap();
    assert_eq!(status, MigrationStatus::Uninitialized);

    // _sqlx_migrations は依然として存在しない（チェックが DDL を打っていない）。
    let missing: bool = sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations') IS NULL")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        missing,
        "check_migration_status がテーブルを作ってはいけない"
    );
}

/// 全適用後に適用済み version を 1 つ削除＝「バイナリが知るが DB 未適用」を作ると Pending。
/// connect_checked(false) が Err 停止する主経路の実証（従来テストに欠けていた単独ケース）。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn missing_applied_version_is_pending(pool: sqlx::PgPool) {
    let removed: i64 = sqlx::query_scalar(
        "DELETE FROM _sqlx_migrations WHERE version = (SELECT MAX(version) FROM _sqlx_migrations) \
         RETURNING version",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let status = pool::check_migration_status(&pool).await.unwrap();
    assert_eq!(status, MigrationStatus::Pending(vec![removed]));
}

/// pending と stale が同時に存在する（別 worktree が別々に migration を足して交差した状態）ときは、
/// Pending を優先して停止側に倒す（自バイナリの未適用 migration があるうちは動かさない）。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn pending_takes_priority_over_stale(pool: sqlx::PgPool) {
    let phantom: i64 = 99990101000000;
    // stale: 埋め込みに無い架空の適用済み。
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES ($1, 'phantom future migration', true, $2, 0)",
    )
    .bind(phantom)
    .bind(vec![0u8; 48])
    .execute(&pool)
    .await
    .unwrap();
    // pending: 実 version（phantom を除く最大）を 1 つ削除。
    let removed: i64 = sqlx::query_scalar(
        "DELETE FROM _sqlx_migrations \
         WHERE version = (SELECT MAX(version) FROM _sqlx_migrations WHERE version <> $1) \
         RETURNING version",
    )
    .bind(phantom)
    .fetch_one(&pool)
    .await
    .unwrap();

    let status = pool::check_migration_status(&pool).await.unwrap();
    assert_eq!(
        status,
        MigrationStatus::Pending(vec![removed]),
        "pending があるうちは stale より優先して停止側（Pending）に倒す"
    );
}

/// dirty 行（`success=false`＝前回失敗）は未適用扱いになり Pending に落ちる（`WHERE success=true` の実証）。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn dirty_row_is_treated_as_unapplied(pool: sqlx::PgPool) {
    let dirtied: i64 = sqlx::query_scalar(
        "UPDATE _sqlx_migrations SET success = false \
         WHERE version = (SELECT MAX(version) FROM _sqlx_migrations) RETURNING version",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let status = pool::check_migration_status(&pool).await.unwrap();
    assert_eq!(status, MigrationStatus::Pending(vec![dirtied]));
}
