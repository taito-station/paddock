use anyhow::Context;
use paddock_config::Config;
use paddock_use_case::Interactor;
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

    fn fetch_if_exists(&self, _url: &str) -> paddock_use_case::Result<Option<Vec<u8>>> {
        Err(paddock_use_case::Error::InvalidArgument(
            "api-server does not fetch PDFs".into(),
        ))
    }
}

/// api-server が DI で組み立てる Interactor の具象型。
pub type ApiInteractor = Interactor<PostgresRepository, UnusedParser, UnusedFetcher>;

pub struct Setup {
    pub interactor: ApiInteractor,
    /// bind アドレス（`host:port`）。
    pub server_addr: String,
}

/// ロガー初期化 → Postgres プール → `PostgresRepository` → `Interactor` を組み立てる。
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
    let repo = PostgresRepository::new(pool);
    let interactor = Interactor::new(repo, UnusedParser, UnusedFetcher);
    Ok(Setup {
        interactor,
        server_addr: config.paddock_server_addr,
    })
}
