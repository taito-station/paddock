//! 予想セッション write API（#53）の統合テスト。`#[sqlx::test]` の一時 Postgres を使う。
//!
//! 実行には Postgres が必要。Postgres 非接続環境ではコンパイルのみ確認できる（`cargo test --no-run`）。
//! odds:refresh / results:refresh は外部スクレイプを伴うためここでは検証しない（ネットワーク必須）。

use actix_web::{App, test, web};
use serde_json::{Value, json};

use api_server::app::configure_routes;
use api_server::setup::{UnusedFetcher, UnusedParser};
use netkeiba_scraper::UreqNetkeibaScraper;
use odds_scraper::UreqOddsScraper;
use paddock_use_case::Interactor;
use rdb_gateway::PostgresRepository;

type Repo = PostgresRepository;

const DATE: &str = "2026-03-28";
const RACE_ID: &str = "2026-1-nakayama-1-R1";

macro_rules! build_service {
    ($pool:expr) => {{
        let interactor =
            Interactor::new(PostgresRepository::new($pool), UnusedParser, UnusedFetcher);
        test::init_service(App::new().app_data(web::Data::new(interactor)).configure(
            configure_routes::<
                Repo,
                UnusedParser,
                UnusedFetcher,
                UreqOddsScraper,
                UreqNetkeibaScraper,
            >,
        ))
        .await
    }};
}

async fn body_json(resp: actix_web::dev::ServiceResponse) -> Value {
    let bytes = test::read_body(resp).await;
    serde_json::from_slice(&bytes).expect("response body is JSON")
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn create_then_summary(pool: sqlx::PgPool) {
    let app = build_service!(pool);

    let req = test::TestRequest::post()
        .uri(&format!("/api/sessions/{DATE}"))
        .set_json(json!({ "budget": 10000 }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 201, "create should be 201");
    let json = body_json(resp).await;
    assert_eq!(json["budget"], 10000);
    assert_eq!(json["balance"], 10000);
    assert_eq!(json["total_bet"], 0);
    assert_eq!(json["pnl"], 0);
    assert_eq!(json["completed"], false);

    let req = test::TestRequest::get()
        .uri(&format!("/api/sessions/{DATE}"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
    let json = body_json(resp).await;
    assert_eq!(json["balance"], 10000);
    assert_eq!(json["bets"].as_array().unwrap().len(), 0);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn double_create_conflicts(pool: sqlx::PgPool) {
    let app = build_service!(pool);
    let create = || {
        test::TestRequest::post()
            .uri(&format!("/api/sessions/{DATE}"))
            .set_json(json!({ "budget": 10000 }))
            .to_request()
    };
    let first = test::call_service(&app, create()).await;
    assert_eq!(first.status().as_u16(), 201);
    let second = test::call_service(&app, create()).await;
    assert_eq!(second.status().as_u16(), 409);
    let json = body_json(second).await;
    assert_eq!(json["error"]["code"], "conflict");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn create_rejects_zero_budget(pool: sqlx::PgPool) {
    let app = build_service!(pool);
    let req = test::TestRequest::post()
        .uri(&format!("/api/sessions/{DATE}"))
        .set_json(json!({ "budget": 0 }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 400);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn outcome_updates_balance_and_summary(pool: sqlx::PgPool) {
    let app = build_service!(pool);
    test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/api/sessions/{DATE}"))
            .set_json(json!({ "budget": 10000 }))
            .to_request(),
    )
    .await;

    let req = test::TestRequest::post()
        .uri(&format!("/api/sessions/{DATE}/races/{RACE_ID}/outcome"))
        .set_json(json!({
            "bets": [
                { "bet_type": "単勝", "combination": "1", "stake": 3000, "payout": 5000, "ev": 1.2 }
            ]
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "status: {}", resp.status());
    let json = body_json(resp).await;
    // 10000 - 3000 + 5000 = 12000
    assert_eq!(json["balance"], 12000);
    assert_eq!(json["total_bet"], 3000);
    assert_eq!(json["total_payout"], 5000);
    assert_eq!(json["pnl"], 2000);
    assert_eq!(json["bets"].as_array().unwrap().len(), 1);
    assert_eq!(json["bets"][0]["race_id"], RACE_ID);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn duplicate_outcome_for_same_race_conflicts(pool: sqlx::PgPool) {
    let app = build_service!(pool);
    test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/api/sessions/{DATE}"))
            .set_json(json!({ "budget": 10000 }))
            .to_request(),
    )
    .await;

    let outcome = || {
        test::TestRequest::post()
            .uri(&format!("/api/sessions/{DATE}/races/{RACE_ID}/outcome"))
            .set_json(json!({
                "bets": [ { "bet_type": "単勝", "combination": "1", "stake": 3000, "payout": 0, "ev": 1.0 } ]
            }))
            .to_request()
    };
    let first = test::call_service(&app, outcome()).await;
    assert!(first.status().is_success());

    // 同一レースへの再記録は 409（買い目重複・残高二重適用を防ぐ）。
    let second = test::call_service(&app, outcome()).await;
    assert_eq!(second.status().as_u16(), 409);
    let json = body_json(second).await;
    assert_eq!(json["error"]["code"], "conflict");

    // 状態不変: 1 回目の控除のみ（10000 - 3000 = 7000）。
    let summary = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/api/sessions/{DATE}"))
            .to_request(),
    )
    .await;
    let json = body_json(summary).await;
    assert_eq!(json["balance"], 7000);
    assert_eq!(json["bets"].as_array().unwrap().len(), 1);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn outcome_rejects_stake_over_balance_without_state_change(pool: sqlx::PgPool) {
    let app = build_service!(pool);
    test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/api/sessions/{DATE}"))
            .set_json(json!({ "budget": 1000 }))
            .to_request(),
    )
    .await;

    let req = test::TestRequest::post()
        .uri(&format!("/api/sessions/{DATE}/races/{RACE_ID}/outcome"))
        .set_json(json!({
            "bets": [ { "bet_type": "単勝", "combination": "1", "stake": 2000, "payout": 0, "ev": 0.0 } ]
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 400);

    // 状態不変: 残高は budget のまま。
    let summary = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/api/sessions/{DATE}"))
            .to_request(),
    )
    .await;
    let json = body_json(summary).await;
    assert_eq!(json["balance"], 1000);
    assert_eq!(json["total_bet"], 0);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn outcome_and_summary_404_when_no_session(pool: sqlx::PgPool) {
    let app = build_service!(pool);

    let summary = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/api/sessions/{DATE}"))
            .to_request(),
    )
    .await;
    assert_eq!(summary.status().as_u16(), 404);

    let outcome = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/api/sessions/{DATE}/races/{RACE_ID}/outcome"))
            .set_json(json!({ "bets": [] }))
            .to_request(),
    )
    .await;
    assert_eq!(outcome.status().as_u16(), 404);
}
