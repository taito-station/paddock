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
