use anyhow::Context;
use paddock_config::Config;
use paddock_use_case::{Interactor, NoopFetcher, NoopParser};
use rdb_gateway::{PostgresRepository, pool};

pub struct App {
    // ingest-predictions は PDF を扱わないため PDF 系ジェネリクスは use-case 共通の Noop スタブ（#410）。
    pub interactor: Interactor<PostgresRepository, NoopParser, NoopFetcher>,
}

pub async fn build_app() -> anyhow::Result<App> {
    let config = Config::from_env().context("load config")?;
    config.init_tracing();

    let pool = pool::connect_and_migrate(&config.paddock_db_url)
        .await
        .context("connect and migrate Postgres")?;
    let repo = PostgresRepository::new(pool);
    let interactor = Interactor::new(repo, NoopParser, NoopFetcher);
    Ok(App { interactor })
}
