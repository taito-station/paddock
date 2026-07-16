use sqlx::postgres::PgPoolOptions;

use crate::error::Result;

pub use sqlx::PgPool;

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
