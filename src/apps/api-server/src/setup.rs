use anyhow::Context;
use netkeiba_scraper::UreqNetkeibaScraper;
use paddock_config::Config;
use paddock_use_case::{Interactor, NoopFetcher, NoopParser, OddsInteractor, SettleInteractor};
use rdb_gateway::{PostgresRepository, pool};

/// api-server が DI で組み立てる Interactor の具象型。read 専用 API で PDF は扱わないため、
/// PDF 系ジェネリクスは use-case 共通の Noop スタブ（#410）。
pub type ApiInteractor = Interactor<PostgresRepository, NoopParser, NoopFetcher>;
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
    config.init_tracing();

    let pool = pool::connect_and_migrate(&config.paddock_db_url)
        .await
        .context("connect and migrate Postgres")?;

    let odds = OddsInteractor::new(
        UreqNetkeibaScraper::new(),
        PostgresRepository::new(pool.clone()),
    );
    let settle = SettleInteractor::new(
        UreqNetkeibaScraper::new(),
        PostgresRepository::new(pool.clone()),
    );
    let interactor = Interactor::new(PostgresRepository::new(pool), NoopParser, NoopFetcher);
    Ok(Setup {
        interactor,
        odds,
        settle,
        server_addr: config.paddock_server_addr,
    })
}
