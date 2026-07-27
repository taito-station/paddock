use std::collections::BTreeSet;

use sqlx::postgres::PgPoolOptions;

use crate::error::{Error, Result};

pub use sqlx::PgPool;

/// 起動時 read-only 整合チェック（#470）の結果。DDL を一切発行せず、埋め込みマイグレーション
/// （バイナリが知る version）と DB 適用済み version（`_sqlx_migrations`）の差から判定する。
#[derive(Debug, PartialEq, Eq)]
pub enum MigrationStatus {
    /// 埋め込みと適用済みが完全一致（過不足なし）。
    UpToDate,
    /// バイナリが知るが DB 未適用の version がある（要 `paddock-analyze migrate`）。
    Pending(Vec<i64>),
    /// DB に適用済みだがバイナリが知らない version がある（＝バイナリが古い可能性）。
    StaleBinary(Vec<i64>),
    /// `_sqlx_migrations` テーブルが無い（未初期化 DB）。
    Uninitialized,
}

pub async fn connect(database_url: &str) -> Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await?;
    Ok(pool)
}

pub async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::migrate!("../../../deployments/db/migrations")
        .run(pool)
        .await?;
    Ok(())
}

/// connect → migrate をまとめて実行し、マイグレーション適用済みの [`PgPool`] を返す（#410）。
/// 全 app の build_app が同一の「接続してからマイグレート」シーケンスを重複していたのを集約する。
pub async fn connect_and_migrate(database_url: &str) -> Result<PgPool> {
    let pool = connect(database_url).await?;
    migrate(&pool).await?;
    Ok(pool)
}

/// 埋め込みマイグレーションと DB 適用済みマイグレーションの差を **DDL を一切発行せず** 調べる（#470）。
///
/// - `Migrator::run` や `ensure_migrations_table` は呼ばない（起動時の無条件 auto-migrate をやめるため、
///   共有 DB に副作用を残さない read-only チェックに徹する）。
/// - `_sqlx_migrations` が無い（未初期化 DB）は SQLSTATE `42P01` を握って [`MigrationStatus::Uninitialized`]。
/// - 適用済みは `success=true` の行のみを数える（dirty＝前回失敗の行は未適用扱い）。
/// - `pending`（バイナリが知るが DB 未適用）を最優先で [`MigrationStatus::Pending`]（当該バイナリのクエリが
///   未適用スキーマを参照して壊れうるため、stale と同時でも停止側に倒す）、次いで `stale`（DB にあるが
///   バイナリが知らない）を [`MigrationStatus::StaleBinary`]、両方空なら [`MigrationStatus::UpToDate`]。
pub async fn check_migration_status(pool: &PgPool) -> Result<MigrationStatus> {
    // バイナリが知る version（up マイグレーションのみ。down は適用単位ではない）。
    let embedded: BTreeSet<i64> = sqlx::migrate!("../../../deployments/db/migrations")
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .map(|m| m.version)
        .collect();

    // DB 適用済み version（`success=true` のみ＝dirty な失敗行は未適用扱い）。
    // テーブル不在（未初期化）は Uninitialized に落とす。
    let applied: BTreeSet<i64> = match sqlx::query_scalar::<_, i64>(
        "SELECT version FROM _sqlx_migrations WHERE success = true ORDER BY version",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows.into_iter().collect(),
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("42P01") => {
            return Ok(MigrationStatus::Uninitialized);
        }
        Err(e) => return Err(e.into()),
    };

    let pending: Vec<i64> = embedded.difference(&applied).copied().collect();
    let stale: Vec<i64> = applied.difference(&embedded).copied().collect();

    // pending（バイナリが知るが DB 未適用）は、そのまま起動すると当該バイナリのクエリが未適用スキーマを
    // 参照して壊れうるため最優先で停止側に倒す（stale と同時でも pending を優先）。次いで stale（DB にのみ
    // 適用済み＝バイナリが古い疑い）は warn 継続にとどめる。両方空なら UpToDate。
    if !pending.is_empty() {
        Ok(MigrationStatus::Pending(pending))
    } else if !stale.is_empty() {
        Ok(MigrationStatus::StaleBinary(stale))
    } else {
        Ok(MigrationStatus::UpToDate)
    }
}

/// 起動時の DB 接続（#470）。既定（`auto_migrate=false`）は **auto-migrate せず** read-only 整合チェックのみ行う。
///
/// - `auto_migrate=true`（prod 経路。compose の `PADDOCK_AUTO_MIGRATE=true`）は従来どおり [`migrate`] を適用する。
/// - `auto_migrate=false` は [`check_migration_status`] で分岐する:
///   - [`MigrationStatus::UpToDate`] → 何もしない。
///   - [`MigrationStatus::StaleBinary`] → warn して **継続**（DB が先行しているだけで、当該バイナリの動作は成立しうる）。
///   - [`MigrationStatus::Pending`] → warn して **`Err` で停止**（未適用のまま動くと不整合。明示適用を促す）。
///   - [`MigrationStatus::Uninitialized`] → warn して **`Err` で停止**（テーブルが無い＝初回セットアップ未実施）。
pub async fn connect_checked(database_url: &str, auto_migrate: bool) -> Result<PgPool> {
    let pool = connect(database_url).await?;
    if auto_migrate {
        migrate(&pool).await?;
        return Ok(pool);
    }

    match check_migration_status(&pool).await? {
        MigrationStatus::UpToDate => {}
        MigrationStatus::StaleBinary(v) => {
            tracing::warn!(
                db_ahead = ?v,
                "このバイナリは古い可能性があります（DB に未知のマイグレーションが適用済み）。最新ブランチで cargo build --release し直してください"
            );
        }
        MigrationStatus::Pending(v) => {
            tracing::warn!(
                pending = ?v,
                "未適用マイグレーションがあります。`paddock-analyze migrate` で適用してください"
            );
            return Err(Error::MigrationRequired(format!(
                "未適用マイグレーションがあります: {v:?}。`paddock-analyze migrate` で適用してください"
            )));
        }
        MigrationStatus::Uninitialized => {
            tracing::warn!("DB 未初期化。`paddock-analyze migrate` で適用してください");
            return Err(Error::MigrationRequired(
                "DB 未初期化。`paddock-analyze migrate` で適用してください".to_string(),
            ));
        }
    }
    Ok(pool)
}
