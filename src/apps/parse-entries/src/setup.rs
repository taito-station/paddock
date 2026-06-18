use anyhow::Context;
use entry_parser::MutoolEntryParser;
use jra_fetcher::JraFetcher;
use paddock_config::Config;
use paddock_use_case::EntryInteractor;
use rdb_gateway::{PostgresRepository, pool};
use tracing_subscriber::{EnvFilter, fmt};

pub type App = EntryInteractor<PostgresRepository, MutoolEntryParser, JraFetcher>;

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
    // Entry fetches are single-shot; no rate cap needed (`None`). Timeouts and
    // retries come from the shared fetcher.
    Ok(EntryInteractor::new(
        repo,
        MutoolEntryParser,
        JraFetcher::new(None),
    ))
}
