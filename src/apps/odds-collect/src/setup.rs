use std::time::Duration;

use anyhow::Context;
use netkeiba_scraper::UreqNetkeibaScraper;
use paddock_config::Config;
use paddock_use_case::{Interactor, OddsInteractor};
use rdb_gateway::{PostgresRepository, pool};
use tracing_subscriber::{EnvFilter, fmt};

/// odds-collect は PDF 解析・取得を使わないため no-op を注入する（predict-watch と同パターン）。
pub struct UnusedParser;

impl paddock_use_case::pdf_parser::PdfParser for UnusedParser {
    fn parse(&self, _bytes: &[u8]) -> paddock_use_case::Result<Vec<paddock_domain::Race>> {
        Err(paddock_use_case::Error::InvalidArgument(
            "odds-collect bin does not parse PDFs".into(),
        ))
    }
}

pub struct UnusedFetcher;

impl paddock_use_case::pdf_fetcher::PdfFetcher for UnusedFetcher {
    fn fetch(&self, _url: &str) -> paddock_use_case::Result<Vec<u8>> {
        Err(paddock_use_case::Error::InvalidArgument(
            "odds-collect bin does not fetch PDFs".into(),
        ))
    }

    fn fetch_if_exists(
        &self,
        _url: &str,
    ) -> paddock_use_case::Result<paddock_use_case::pdf_fetcher::FetchProbe> {
        Err(paddock_use_case::Error::InvalidArgument(
            "odds-collect bin does not fetch PDFs".into(),
        ))
    }
}

/// 収集に必要な依存だけを束ねる。predict/EV/買い目・セッション記録の interactor を呼ばない
/// ＝**確率（モデル）と収集を分離**した純粋なデータ収集（predict_sessions/predict_bets に触れない）。
/// races_by_date/race_card（発走時刻）と単複オッズの再取得・保存のみを行う。
pub struct App {
    pub interactor: Interactor<PostgresRepository, UnusedParser, UnusedFetcher>,
    /// オッズは `refresh_win_place_odds` で**毎回再スクレイプ**して新スナップショットを貯める。
    pub odds: OddsInteractor<UreqNetkeibaScraper, PostgresRepository>,
}

/// `scrape_delay_ms` はオッズスクレイパの 1 リクエストごとの待機（netkeiba への礼節・[[jra-fetch-pacing]]）。
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
        UreqNetkeibaScraper::with_delay(Duration::from_millis(scrape_delay_ms)),
        PostgresRepository::new(pool.clone()),
    );
    let interactor = Interactor::new(PostgresRepository::new(pool), UnusedParser, UnusedFetcher);
    Ok(App { interactor, odds })
}
