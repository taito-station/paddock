use std::time::Duration;

use anyhow::Context;
use netkeiba_scraper::UreqNetkeibaScraper;
use paddock_config::Config;
use rdb_gateway::{PostgresRepository, pool};
use tracing_subscriber::{EnvFilter, fmt};

pub async fn build(
    interval_ms: Option<u64>,
) -> anyhow::Result<(PostgresRepository, UreqNetkeibaScraper)> {
    let config = Config::from_env().context("load config")?;
    let _ = fmt()
        .with_env_filter(
            EnvFilter::try_new(config.paddock_log.clone())
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .try_init();

    let pool = pool::connect(&config.paddock_db_url)
        .await
        .context("connect Postgres pool")?;
    pool::migrate(&pool).await.context("apply migrations")?;
    let repo = PostgresRepository::new(pool);

    let scraper = match interval_ms {
        Some(ms) => UreqNetkeibaScraper::with_delay(Duration::from_millis(ms)),
        None => UreqNetkeibaScraper::new(),
    };
    Ok((repo, scraper))
}
