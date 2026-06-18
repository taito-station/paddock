use std::time::Duration;

use anyhow::Context;
use jra_fetcher::JraFetcher;
use paddock_config::Config;
use paddock_use_case::Interactor;
use pdf_parser::HybridParser;
use rdb_gateway::{PostgresRepository, pool};
use tracing_subscriber::{EnvFilter, fmt};

pub type App = Interactor<PostgresRepository, HybridParser, JraFetcher>;

/// Build the app. `fetch_min_interval` sets a global minimum spacing between
/// outbound JRA requests (from `fetch --max-rps`); `None` imposes no cap.
pub async fn build_app(fetch_min_interval: Option<Duration>) -> anyhow::Result<App> {
    let config = Config::from_env().context("load config")?;
    let _ = fmt()
        .with_env_filter(
            EnvFilter::try_new(config.paddock_log.clone())
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .try_init();

    pdf_ocr::ensure_available("jpn").context("tesseract preflight")?;

    let pool = pool::connect(&config.paddock_db_url)
        .await
        .context("connect Postgres pool")?;
    pool::migrate(&pool).await.context("apply migrations")?;
    let repo = PostgresRepository::new(pool);
    Ok(Interactor::new(
        repo,
        HybridParser::new(),
        JraFetcher::new(fetch_min_interval),
    ))
}
