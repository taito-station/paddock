use anyhow::Context;
use netkeiba_scraper::UreqNetkeibaScraper;
use paddock_config::Config;
use paddock_use_case::{Interactor, OddsInteractor, ResultsInteractor};
use rdb_gateway::{PostgresRepository, pool};

/// api-server が DI で組み立てる Interactor の具象型。read 専用 API で PDF は扱わないため、
/// PDF 系ユースケース（`PdfInteractor`）は持たず Repository のみ（#453 で P/F ジェネリクスを解消）。
pub type ApiInteractor = Interactor<PostgresRepository>;
/// オッズ read-through 取得用（#51, odds:refresh）。
pub type ApiOddsInteractor = OddsInteractor<UreqNetkeibaScraper, PostgresRepository>;
/// 同日結果取り込み＋自動精算用（#381, results:refresh）。`UreqNetkeibaScraper` が `ResultPageFetcher`。
pub type ApiResultsInteractor = ResultsInteractor<UreqNetkeibaScraper, PostgresRepository>;

pub struct Setup {
    pub interactor: ApiInteractor,
    pub odds: ApiOddsInteractor,
    pub results: ApiResultsInteractor,
    /// bind アドレス（`host:port`）。
    pub server_addr: String,
}

/// ロガー初期化 → Postgres プール → 各 Interactor を組み立てる。
/// プールは sqlx の Arc ベースで安価に clone でき、read/odds/results で共有する（predict と同流儀）。
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
        server_addr: config.paddock_server_addr,
    })
}
