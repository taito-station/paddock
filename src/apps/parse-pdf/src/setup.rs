use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use jra_fetcher::JraFetcher;
use paddock_config::Config;
use paddock_use_case::Interactor;
use pdf_parser::HybridParser;
use rdb_gateway::{PostgresRepository, pool};

pub type App = Interactor<PostgresRepository, HybridParser, JraFetcher>;

/// The built app together with the configured PDF root (`paddock_pdfs_dir`), from
/// which Stage1 derives the results inbox dir (`<root>/results/inbox`).
pub struct Built {
    pub app: App,
    pub pdfs_dir: PathBuf,
}

/// Build the app. `fetch_min_interval` sets a global minimum spacing between
/// outbound JRA requests (from `fetch --max-rps`); `None` imposes no cap.
pub async fn build_app(fetch_min_interval: Option<Duration>) -> anyhow::Result<Built> {
    let config = Config::from_env().context("load config")?;
    config.init_tracing();

    pdf_ocr::ensure_available("jpn").context("tesseract preflight")?;

    let pool = pool::connect_and_migrate(&config.paddock_db_url)
        .await
        .context("connect and migrate Postgres")?;
    let repo = PostgresRepository::new(pool);
    let app = Interactor::new(
        repo,
        HybridParser::new(),
        JraFetcher::new(fetch_min_interval),
    );
    Ok(Built {
        app,
        pdfs_dir: PathBuf::from(config.paddock_pdfs_dir),
    })
}
