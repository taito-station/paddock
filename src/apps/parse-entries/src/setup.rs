use std::io::Read;

use anyhow::Context;
use entry_parser::MutoolEntryParser;
use paddock_config::Config;
use paddock_use_case::EntryInteractor;
use paddock_use_case::pdf_fetcher::PdfFetcher;
use rdb_gateway::{SqliteRepository, pool};
use tracing_subscriber::{EnvFilter, fmt};

pub struct UreqFetcher;

impl PdfFetcher for UreqFetcher {
    fn fetch(&self, url: &str) -> paddock_use_case::Result<Vec<u8>> {
        let resp = ureq::get(url)
            .call()
            .map_err(|e| paddock_use_case::Error::Internal(format!("fetch {url}: {e}")))?;
        let mut buf = Vec::new();
        resp.into_reader()
            .read_to_end(&mut buf)
            .map_err(|e| paddock_use_case::Error::Internal(format!("read body: {e}")))?;
        Ok(buf)
    }
}

pub type App = EntryInteractor<SqliteRepository, MutoolEntryParser, UreqFetcher>;

pub async fn build_app() -> anyhow::Result<App> {
    let config = Config::from_env().context("load config")?;
    let _ = fmt()
        .with_env_filter(
            EnvFilter::try_new(config.paddock_log.clone())
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .try_init();

    ensure_data_dir(&config.paddock_db_url)?;

    let pool = pool::connect(&config.paddock_db_url)
        .await
        .context("connect SQLite pool")?;
    pool::migrate(&pool).await.context("apply migrations")?;
    let repo = SqliteRepository::new(pool);
    Ok(EntryInteractor::new(repo, MutoolEntryParser, UreqFetcher))
}

fn ensure_data_dir(database_url: &str) -> anyhow::Result<()> {
    if let Some(rest) = database_url.strip_prefix("sqlite://") {
        let path = rest.split('?').next().unwrap_or(rest);
        if let Some(parent) = std::path::Path::new(path).parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create db parent dir {}", parent.display()))?;
        }
    }
    Ok(())
}
