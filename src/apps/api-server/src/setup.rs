use anyhow::Context;
use netkeiba_scraper::UreqNetkeibaScraper;
use paddock_config::Config;
use paddock_use_case::{Interactor, OddsInteractor, SettleInteractor};
use rdb_gateway::{PostgresRepository, pool};
use tracing_subscriber::{EnvFilter, fmt};

/// read 専用 API なので PDF の parse/fetch は使わないが、`Interactor<R, P, F>` の
/// 型を満たすためのスタブ（analyze/predict と同じ流儀）。呼ばれたら明示エラーを返す。
pub struct UnusedParser;

impl paddock_use_case::pdf_parser::PdfParser for UnusedParser {
    fn parse(&self, _bytes: &[u8]) -> paddock_use_case::Result<Vec<paddock_domain::Race>> {
        Err(paddock_use_case::Error::InvalidArgument(
            "api-server does not parse PDFs".into(),
        ))
    }
}

pub struct UnusedFetcher;

impl paddock_use_case::pdf_fetcher::PdfFetcher for UnusedFetcher {
    fn fetch(&self, _url: &str) -> paddock_use_case::Result<Vec<u8>> {
        Err(paddock_use_case::Error::InvalidArgument(
            "api-server does not fetch PDFs".into(),
        ))
    }

    fn fetch_if_exists(
        &self,
        _url: &str,
    ) -> paddock_use_case::Result<paddock_use_case::pdf_fetcher::FetchProbe> {
        Err(paddock_use_case::Error::InvalidArgument(
            "api-server does not fetch PDFs".into(),
        ))
    }
}

/// api-server が DI で組み立てる Interactor の具象型。
pub type ApiInteractor = Interactor<PostgresRepository, UnusedParser, UnusedFetcher>;
/// オッズ read-through 取得用（#51, odds:refresh）。
pub type ApiOddsInteractor = OddsInteractor<UreqNetkeibaScraper, PostgresRepository>;
/// 確定払戻の自動精算用（#40, results:refresh）。
pub type ApiSettleInteractor = SettleInteractor<UreqNetkeibaScraper, PostgresRepository>;

pub struct Setup {
    pub interactor: ApiInteractor,
    pub odds: ApiOddsInteractor,
    pub settle: ApiSettleInteractor,
    /// bind アドレス（`host:port`）。
    pub server_addr: String,
}

/// ロガー初期化 → Postgres プール → 各 Interactor を組み立てる。
/// プールは sqlx の Arc ベースで安価に clone でき、read/odds/settle で共有する（predict と同流儀）。
pub async fn build() -> anyhow::Result<Setup> {
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

    let odds = OddsInteractor::new(
        UreqNetkeibaScraper::new(),
        PostgresRepository::new(pool.clone()),
    );
    let settle = SettleInteractor::new(
        UreqNetkeibaScraper::new(),
        PostgresRepository::new(pool.clone()),
    );
    let interactor = Interactor::new(PostgresRepository::new(pool), UnusedParser, UnusedFetcher);
    Ok(Setup {
        interactor,
        odds,
        settle,
        server_addr: config.paddock_server_addr,
    })
}
