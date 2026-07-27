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
