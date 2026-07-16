use anyhow::Context;
use entry_parser::MutoolEntryParser;
use jra_fetcher::JraFetcher;
use paddock_config::Config;
use paddock_use_case::EntryInteractor;
use rdb_gateway::{PostgresRepository, pool};

pub type App = EntryInteractor<PostgresRepository, MutoolEntryParser, JraFetcher>;

pub async fn build_app() -> anyhow::Result<App> {
    let config = Config::from_env().context("load config")?;
    config.init_tracing();

    let pool = pool::connect_and_migrate(&config.paddock_db_url)
        .await
        .context("connect and migrate Postgres")?;
    let repo = PostgresRepository::new(pool);
    // Entry fetches are single-shot; no rate cap needed (`None`). Timeouts and
    // retries come from the shared fetcher.
    Ok(EntryInteractor::new(
        repo,
        MutoolEntryParser,
        JraFetcher::new(None),
    ))
}
