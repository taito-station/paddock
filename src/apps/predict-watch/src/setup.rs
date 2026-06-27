use std::time::Duration;

use anyhow::Context;
use odds_scraper::UreqOddsScraper;
use paddock_config::Config;
use paddock_use_case::{Interactor, OddsInteractor};
use rdb_gateway::{PostgresRepository, pool};
use tracing_subscriber::{EnvFilter, fmt};

/// predict-watch は PDF 解析・取得を使わないため no-op を注入する（predict/analyze と同パターン）。
pub struct UnusedParser;

impl paddock_use_case::pdf_parser::PdfParser for UnusedParser {
    fn parse(&self, _bytes: &[u8]) -> paddock_use_case::Result<Vec<paddock_domain::Race>> {
        Err(paddock_use_case::Error::InvalidArgument(
            "predict-watch bin does not parse PDFs".into(),
        ))
    }
}

pub struct UnusedFetcher;

impl paddock_use_case::pdf_fetcher::PdfFetcher for UnusedFetcher {
    fn fetch(&self, _url: &str) -> paddock_use_case::Result<Vec<u8>> {
        Err(paddock_use_case::Error::InvalidArgument(
            "predict-watch bin does not fetch PDFs".into(),
        ))
    }

    fn fetch_if_exists(
        &self,
        _url: &str,
    ) -> paddock_use_case::Result<paddock_use_case::pdf_fetcher::FetchProbe> {
        Err(paddock_use_case::Error::InvalidArgument(
            "predict-watch bin does not fetch PDFs".into(),
        ))
    }
}

/// 監視に必要な依存だけを束ねる。記録系（settle 等）は持たない＝**読み取り専用**を構造で担保する。
pub struct App {
    pub interactor: Interactor<PostgresRepository, UnusedParser, UnusedFetcher>,
    /// オッズは `refresh_race_odds` で**毎回再スクレイプ**する（read-through キャッシュは使わない、#257）。
    pub odds: OddsInteractor<UreqOddsScraper, PostgresRepository>,
}

/// `scrape_delay_ms` はオッズスクレイパの 1 リクエストごとの待機（JRA への礼節, [[jra-fetch-pacing]]）。
pub async fn build_app(scrape_delay_ms: u64) -> anyhow::Result<App> {
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

    // PgPool は Arc backed なので clone は安価。odds / interactor で同一 DB を共有する。
    let odds = OddsInteractor::new(
        UreqOddsScraper::with_delay(Duration::from_millis(scrape_delay_ms)),
        PostgresRepository::new(pool.clone()),
    );
    let interactor = Interactor::new(PostgresRepository::new(pool), UnusedParser, UnusedFetcher);
    Ok(App { interactor, odds })
}
