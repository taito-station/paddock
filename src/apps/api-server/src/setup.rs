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
    // 間隔の指定（#721）は DB に接続する前に検証する（設定の誤りで副作用を起こさない）。
    // 指定があればそれを使い、無ければ scraper の既定。既定より短い指定は起動エラー。
    let interval = config.netkeiba_interval().context("netkeiba 取得間隔")?;
    match interval {
        Some(d) => tracing::info!(
            interval_ms = d.as_millis() as u64,
            "netkeiba 取得間隔を指定値にします（odds / results とも）"
        ),
        None => tracing::info!("netkeiba 取得間隔は scraper の既定"),
    }

    let pool = pool::connect_checked(&config.paddock_db_url, config.paddock_auto_migrate)
        .await
        .context("connect Postgres")?;

    let scraper = || match interval {
        Some(d) => UreqNetkeibaScraper::with_delay(d),
        None => UreqNetkeibaScraper::new(),
    };
    let odds = OddsInteractor::new(scraper(), PostgresRepository::new(pool.clone()));
    let results = ResultsInteractor::new(scraper(), PostgresRepository::new(pool.clone()));
    let interactor = Interactor::new(PostgresRepository::new(pool));
    Ok(Setup {
        interactor,
        odds,
        results,
        server_addr: config.paddock_server_addr,
    })
}
