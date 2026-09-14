use anyhow::Context;
use netkeiba_scraper::UreqNetkeibaScraper;
use paddock_config::Config;
use paddock_use_case::{Interactor, OddsInteractor, ResultsInteractor};
use rdb_gateway::{PostgresRepository, pool};

pub type DesktopInteractor = Interactor<PostgresRepository>;
pub type DesktopOddsInteractor = OddsInteractor<UreqNetkeibaScraper, PostgresRepository>;
pub type DesktopResultsInteractor = ResultsInteractor<UreqNetkeibaScraper, PostgresRepository>;

pub struct Setup {
    pub interactor: DesktopInteractor,
    pub odds: DesktopOddsInteractor,
    pub results: DesktopResultsInteractor,
}

pub async fn build() -> anyhow::Result<Setup> {
    let config = Config::from_env().context("load config")?;
    config.init_tracing();
    let pool = pool::connect_checked(&config.paddock_db_url, config.paddock_auto_migrate)
        .await
        .context("connect Postgres")?;
    let odds = OddsInteractor::new(
        UreqNetkeibaScraper::new(),
        PostgresRepository::new(pool.clone()),
    );
    let results = ResultsInteractor::new(
        UreqNetkeibaScraper::new(),
        PostgresRepository::new(pool.clone()),
    );
    let interactor = Interactor::new(PostgresRepository::new(pool));
    Ok(Setup {
        interactor,
        odds,
        results,
    })
}
