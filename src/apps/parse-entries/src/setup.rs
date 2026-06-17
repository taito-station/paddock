use std::io::Read;

use anyhow::Context;
use entry_parser::MutoolEntryParser;
use paddock_config::Config;
use paddock_use_case::EntryInteractor;
use paddock_use_case::pdf_fetcher::PdfFetcher;
use rdb_gateway::{PostgresRepository, pool};
use tracing_subscriber::{EnvFilter, fmt};

pub struct UreqFetcher;

impl PdfFetcher for UreqFetcher {
    fn fetch(&self, url: &str) -> paddock_use_case::Result<Vec<u8>> {
        let resp = ureq::get(url)
            .call()
            .map_err(|e| paddock_use_case::Error::Internal(format!("fetch {url}: {e}")))?;
        read_body(url, resp.into_body())
    }

    fn fetch_if_exists(&self, url: &str) -> paddock_use_case::Result<Option<Vec<u8>>> {
        match ureq::get(url).call() {
            Ok(resp) => Ok(Some(read_body(url, resp.into_body())?)),
            // 404 means the resource is not published (yet); treat as absent.
            Err(ureq::Error::StatusCode(404)) => Ok(None),
            Err(e) => Err(paddock_use_case::Error::Internal(format!(
                "fetch {url}: {e}"
            ))),
        }
    }
}

fn read_body(url: &str, body: ureq::Body) -> paddock_use_case::Result<Vec<u8>> {
    let mut buf = Vec::new();
    body.into_reader()
        .read_to_end(&mut buf)
        .map_err(|e| paddock_use_case::Error::Internal(format!("read body {url}: {e}")))?;
    Ok(buf)
}

pub type App = EntryInteractor<PostgresRepository, MutoolEntryParser, UreqFetcher>;

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
    Ok(EntryInteractor::new(repo, MutoolEntryParser, UreqFetcher))
}
