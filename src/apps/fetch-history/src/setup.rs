use anyhow::Context;
use netkeiba_scraper::UreqNetkeibaScraper;
use paddock_config::Config;
use paddock_use_case::HorseHistoryInteractor;
use rdb_gateway::{PostgresRepository, pool};
use tracing_subscriber::{EnvFilter, fmt};

pub type App = HorseHistoryInteractor<PostgresRepository, UreqNetkeibaScraper>;

pub async fn build_app() -> anyhow::Result<App> {
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
    Ok(HorseHistoryInteractor::new(
        repo,
        UreqNetkeibaScraper::new(),
    ))
}
