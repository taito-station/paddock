use std::time::Duration;

use anyhow::Context;
use netkeiba_scraper::UreqNetkeibaScraper;
use paddock_config::Config;
use rdb_gateway::{PostgresRepository, pool};

pub async fn build(
    interval_ms: Option<u64>,
) -> anyhow::Result<(PostgresRepository, UreqNetkeibaScraper)> {
    let config = Config::from_env().context("load config")?;
    config.init_tracing();

    let pool = pool::connect_and_migrate(&config.paddock_db_url)
        .await
        .context("connect and migrate Postgres")?;
    let repo = PostgresRepository::new(pool);

    let scraper = match interval_ms {
        Some(ms) => UreqNetkeibaScraper::with_delay(Duration::from_millis(ms)),
        None => UreqNetkeibaScraper::new(),
    };
    Ok((repo, scraper))
}
