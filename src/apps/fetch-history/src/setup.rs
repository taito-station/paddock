use anyhow::Context;
use netkeiba_scraper::UreqNetkeibaScraper;
use paddock_config::Config;
use paddock_use_case::HorseHistoryInteractor;
use rdb_gateway::{SqliteRepository, pool};
use tracing_subscriber::{EnvFilter, fmt};

pub type App = HorseHistoryInteractor<SqliteRepository, UreqNetkeibaScraper>;

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
    Ok(HorseHistoryInteractor::new(
        repo,
        UreqNetkeibaScraper::new(),
    ))
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
