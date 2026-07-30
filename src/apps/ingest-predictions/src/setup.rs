use anyhow::Context;
use paddock_config::Config;
use paddock_use_case::Interactor;
use rdb_gateway::{PostgresRepository, pool};

pub struct App {
    pub interactor: Interactor<PostgresRepository>,
}

pub async fn build_app() -> anyhow::Result<App> {
    let config = Config::from_env().context("load config")?;
    config.init_tracing();

    let pool = pool::connect_checked(&config.paddock_db_url, config.paddock_auto_migrate)
        .await
        .context("connect Postgres")?;
    let repo = PostgresRepository::new(pool);
    let interactor = Interactor::new(repo);
    Ok(App { interactor })
}
