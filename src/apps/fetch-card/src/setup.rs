use std::time::Duration;

use anyhow::Context;
use netkeiba_scraper::UreqNetkeibaScraper;
use paddock_config::Config;
use paddock_use_case::{CardInteractor, HorseHistoryInteractor};
use rdb_gateway::{PostgresRepository, pool};
use tracing_subscriber::{EnvFilter, fmt};

/// fetch-card が合成する 2 つの interactor。出馬表・オッズ（card）と、
/// 出走各馬の過去走取り込み（history）を同じ DB プール／スクレイパ設定で束ねる。
pub struct App {
    pub card: CardInteractor<PostgresRepository, UreqNetkeibaScraper>,
    pub history: HorseHistoryInteractor<PostgresRepository, UreqNetkeibaScraper>,
}

pub async fn build_app(interval_ms: Option<u64>) -> anyhow::Result<App> {
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

    // PgPool は Arc backed なので clone は安価。card / history で同一 DB を共有する。
    let card = CardInteractor::new(
        PostgresRepository::new(pool.clone()),
        build_scraper(interval_ms),
    );
    let history =
        HorseHistoryInteractor::new(PostgresRepository::new(pool), build_scraper(interval_ms));
    Ok(App { card, history })
}

/// card / history のスクレイパを同じ間隔設定で生成し、netkeiba への礼節を揃える。
fn build_scraper(interval_ms: Option<u64>) -> UreqNetkeibaScraper {
    match interval_ms {
        Some(ms) => UreqNetkeibaScraper::with_delay(Duration::from_millis(ms)),
        None => UreqNetkeibaScraper::new(),
    }
}
