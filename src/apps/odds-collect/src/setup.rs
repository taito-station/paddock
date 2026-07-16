use std::time::Duration;

use anyhow::Context;
use netkeiba_scraper::UreqNetkeibaScraper;
use paddock_config::Config;
use paddock_use_case::{Interactor, NoopFetcher, NoopParser, OddsInteractor};
use rdb_gateway::{PostgresRepository, pool};

/// 収集に必要な依存だけを束ねる。predict/EV/買い目・セッション記録の interactor を呼ばない
/// ＝**確率（モデル）と収集を分離**した純粋なデータ収集（predict_sessions/predict_bets に触れない）。
/// races_by_date/race_card（発走時刻）と単複オッズの再取得・保存のみを行う。
pub struct App {
    // odds-collect bin は PDF を扱わないため PDF 系ジェネリクスは use-case 共通の Noop スタブ（#410）。
    pub interactor: Interactor<PostgresRepository, NoopParser, NoopFetcher>,
    /// オッズは `refresh_win_place_odds` で**毎回再スクレイプ**して新スナップショットを貯める。
    pub odds: OddsInteractor<UreqNetkeibaScraper, PostgresRepository>,
}

/// `scrape_delay_ms` はオッズスクレイパの 1 リクエストごとの待機（netkeiba への礼節・[[jra-fetch-pacing]]）。
pub async fn build_app(scrape_delay_ms: u64) -> anyhow::Result<App> {
    let config = Config::from_env().context("load config")?;
    config.init_tracing();

    let pool = pool::connect_and_migrate(&config.paddock_db_url)
        .await
        .context("connect and migrate Postgres")?;

    // PgPool は Arc backed なので clone は安価。odds / interactor で同一 DB を共有する。
    let odds = OddsInteractor::new(
        UreqNetkeibaScraper::with_delay(Duration::from_millis(scrape_delay_ms)),
        PostgresRepository::new(pool.clone()),
    );
    let interactor = Interactor::new(PostgresRepository::new(pool), NoopParser, NoopFetcher);
    Ok(App { interactor, odds })
}
