use anyhow::Context;
use netkeiba_scraper::UreqNetkeibaScraper;
use paddock_config::Config;
use paddock_use_case::{Interactor, NoopFetcher, NoopParser, OddsInteractor, SettleInteractor};
use rdb_gateway::{PostgresRepository, pool};

pub struct App {
    // predict bin は PDF を扱わないため PDF 系ジェネリクスは use-case 共通の Noop スタブ（#410）。
    pub interactor: Interactor<PostgresRepository, NoopParser, NoopFetcher>,
    /// オッズは read-through で取得する（保存済み参照 → 無ければスクレイプして保存、#51/ADR 0010）。
    pub odds: OddsInteractor<UreqNetkeibaScraper, PostgresRepository>,
    /// 確定払戻の自動精算（#40、`--settle`）。netkeiba 結果ページから払戻を取得する。
    pub settle: SettleInteractor<UreqNetkeibaScraper, PostgresRepository>,
}

pub async fn build_app() -> anyhow::Result<App> {
    let config = Config::from_env().context("load config")?;
    config.init_tracing();

    let pool = pool::connect_and_migrate(&config.paddock_db_url)
        .await
        .context("connect and migrate Postgres")?;
    // オッズ参照用にプールを共有する（sqlx の PgPool は Arc ベースで安価に clone 可能）。
    let odds = OddsInteractor::new(
        UreqNetkeibaScraper::new(),
        PostgresRepository::new(pool.clone()),
    );
    let settle = SettleInteractor::new(
        UreqNetkeibaScraper::new(),
        PostgresRepository::new(pool.clone()),
    );
    let repo = PostgresRepository::new(pool);
    let interactor = Interactor::new(repo, NoopParser, NoopFetcher);
    Ok(App {
        interactor,
        odds,
        settle,
    })
}
