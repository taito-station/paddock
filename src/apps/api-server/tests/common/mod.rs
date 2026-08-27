//! テスト用 App 構築・共通ヘルパー（#620）。
//!
//! Rust の統合テストは 1 ファイル 1 クレートのため `use` でシンボルを共有できない。
//! 各ファイルが `#[macro_use] mod common;` でこのモジュールを取り込む。
//!
//! **seed（テストデータ投入）は各ファイルに置く**——`prediction-search-api.md` の
//! テスト方針は「共通の helper/mod.rs は置かない」と定めており、対象は seed。
//! ここに置くのは **App 構築（`build_service!`）と汎用ヘルパー（`body_json`）だけ**。

/// テスト用 actix App を組み立てる。`init_service` の戻り値は名前で書けない型なので
/// マクロにする。
///
/// # 2 系統の呼び出し
///
/// - `build_service!($pool)` — `#[sqlx::test]` 由来の実プール。
/// - `build_service!()` — **到達不能アドレス + 1 秒 timeout の遅延プール**。
///   「DB を触らないこと」をテストが強制する契約（`docs_ui.rs` 参照）。
macro_rules! build_service {
    ($pool:expr) => {{
        let interactor = actix_web::web::Data::new(paddock_use_case::Interactor::new(
            rdb_gateway::PostgresRepository::new($pool),
        ));
        actix_web::test::init_service(actix_web::App::new().app_data(interactor).configure(
            api_server::app::configure_routes::<
                rdb_gateway::PostgresRepository,
                netkeiba_scraper::UreqNetkeibaScraper,
                netkeiba_scraper::UreqNetkeibaScraper,
            >,
        ))
        .await
    }};
    () => {{
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_secs(1))
            .connect_lazy("postgres://unused@127.0.0.1:1/unused")
            .expect("build lazy pool");
        build_service!(pool)
    }};
}

#[allow(dead_code)]
pub async fn body_json(resp: actix_web::dev::ServiceResponse) -> serde_json::Value {
    let bytes = actix_web::test::read_body(resp).await;
    serde_json::from_slice(&bytes).expect("response body is JSON")
}
