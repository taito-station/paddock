use anyhow::Context;
use netkeiba_scraper::UreqNetkeibaScraper;
use odds_scraper::UreqOddsScraper;
use paddock_config::Config;
use paddock_use_case::{Interactor, OddsInteractor, SettleInteractor};
use rdb_gateway::{SqliteRepository, pool};
use tracing_subscriber::{EnvFilter, fmt};

/// predict バイナリは PDF 解析・取得を使わないため no-op を注入する（analyze と同パターン）。
pub struct UnusedParser;

impl paddock_use_case::pdf_parser::PdfParser for UnusedParser {
    fn parse(&self, _bytes: &[u8]) -> paddock_use_case::Result<Vec<paddock_domain::Race>> {
        Err(paddock_use_case::Error::InvalidArgument(
            "predict bin does not parse PDFs".into(),
        ))
    }
}

pub struct UnusedFetcher;

impl paddock_use_case::pdf_fetcher::PdfFetcher for UnusedFetcher {
    fn fetch(&self, _url: &str) -> paddock_use_case::Result<Vec<u8>> {
        Err(paddock_use_case::Error::InvalidArgument(
            "predict bin does not fetch PDFs".into(),
        ))
    }

    fn fetch_if_exists(&self, _url: &str) -> paddock_use_case::Result<Option<Vec<u8>>> {
        Err(paddock_use_case::Error::InvalidArgument(
            "predict bin does not fetch PDFs".into(),
        ))
    }
}

pub struct App {
    pub interactor: Interactor<SqliteRepository, UnusedParser, UnusedFetcher>,
    /// オッズは read-through で取得する（保存済み参照 → 無ければスクレイプして保存、#51/ADR 0010）。
    pub odds: OddsInteractor<UreqOddsScraper, SqliteRepository>,
    /// 確定払戻の自動精算（#40、`--settle`）。netkeiba 結果ページから払戻を取得する。
    pub settle: SettleInteractor<UreqNetkeibaScraper, SqliteRepository>,
}

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
        .context("connect SQLite pool")?;
    pool::migrate(&pool).await.context("apply migrations")?;
    // オッズ参照用にプールを共有する（sqlx の SqlitePool は Arc ベースで安価に clone 可能）。
    let odds = OddsInteractor::new(UreqOddsScraper::new(), SqliteRepository::new(pool.clone()));
    let settle = SettleInteractor::new(
        UreqNetkeibaScraper::new(),
        SqliteRepository::new(pool.clone()),
    );
    let repo = SqliteRepository::new(pool);
    let interactor = Interactor::new(repo, UnusedParser, UnusedFetcher);
    Ok(App {
        interactor,
        odds,
        settle,
    })
}
