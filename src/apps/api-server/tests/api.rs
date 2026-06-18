//! REST API（read, #33）の統合テスト。`#[sqlx::test]` の一時 Postgres DB を seed し、
//! 各エンドポイントを actix のテストサーバ越しに叩く。
//!
//! 実行には Postgres が必要（`#[sqlx::test]` が一時 DB を作る）。`cargo test -p api-server`。
//! Postgres 非接続環境ではコンパイルのみ確認できる（`cargo test --no-run`）。

use actix_web::{App, test, web};
use chrono::NaiveDate;
use serde_json::Value;

use api_server::app::configure_routes;
use api_server::setup::{UnusedFetcher, UnusedParser};
use netkeiba_scraper::UreqNetkeibaScraper;
use odds_scraper::UreqOddsScraper;
use paddock_domain::{
    GateNum, HorseEntry, HorseName, HorseNum, Race, RaceCard, RaceId, Surface, Venue,
};
use paddock_use_case::Interactor;
use paddock_use_case::repository::{RaceCardRepository, RaceRepository};
use rdb_gateway::PostgresRepository;

type Repo = PostgresRepository;

const DATE: (i32, u32, u32) = (2026, 3, 28);
const RACE_ID: &str = "2026-1-nakayama-1-R1";

fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(DATE.0, DATE.1, DATE.2).unwrap()
}

fn sample_race() -> Race {
    Race {
        race_id: RaceId::try_from(RACE_ID).unwrap(),
        date: date(),
        venue: Venue::Nakayama,
        round: 1,
        day: 1,
        race_num: 1,
        surface: Surface::Turf,
        distance: 1800,
        track_condition: None,
        weather: None,
        results: Vec::new(),
    }
}

fn entry(gate: u32, num: u32, name: &str) -> HorseEntry {
    HorseEntry {
        gate_num: GateNum::try_from(gate).unwrap(),
        horse_num: HorseNum::try_from(num).unwrap(),
        horse_name: HorseName::try_from(name).unwrap(),
        jockey: None,
        trainer: None,
        weight_carried: None,
    }
}

fn sample_card() -> RaceCard {
    RaceCard {
        race_id: RaceId::try_from(RACE_ID).unwrap(),
        date: date(),
        venue: Venue::Nakayama,
        round: 1,
        day: 1,
        race_num: 1,
        surface: Surface::Turf,
        distance: 1800,
        entries: vec![
            entry(1, 1, "ウマA"),
            entry(2, 2, "ウマB"),
            entry(3, 3, "ウマC"),
        ],
    }
}

/// テスト用 actix App を組み立てる。
macro_rules! build_service {
    ($pool:expr) => {{
        let repo = PostgresRepository::new($pool);
        let interactor = Interactor::new(repo, UnusedParser, UnusedFetcher);
        let data = web::Data::new(interactor);
        test::init_service(App::new().app_data(data).configure(
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
async fn list_races_returns_seeded_race(pool: sqlx::PgPool) {
    PostgresRepository::new(pool.clone())
        .save_race(&sample_race())
        .await
        .unwrap();
    let app = build_service!(pool);

    let req = test::TestRequest::get()
        .uri("/api/races?date=2026-03-28")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "status: {}", resp.status());

    let json = body_json(resp).await;
    assert_eq!(json["date"], "2026-03-28");
    let races = json["races"].as_array().unwrap();
    assert_eq!(races.len(), 1);
    assert_eq!(races[0]["race_id"], RACE_ID);
    assert_eq!(races[0]["venue"], "nakayama");
    assert_eq!(races[0]["surface"], "turf");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn list_races_rejects_bad_date(pool: sqlx::PgPool) {
    let app = build_service!(pool);
    let req = test::TestRequest::get()
        .uri("/api/races?date=2026-13-99")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 400);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "bad_request");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn race_card_found_and_not_found(pool: sqlx::PgPool) {
    PostgresRepository::new(pool.clone())
        .save_race_card(&sample_card())
        .await
        .unwrap();
    let app = build_service!(pool);

    let req = test::TestRequest::get()
        .uri(&format!("/api/races/{RACE_ID}"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
    let json = body_json(resp).await;
    assert_eq!(json["entries"].as_array().unwrap().len(), 3);

    let req = test::TestRequest::get()
        .uri("/api/races/2026-1-nakayama-1-R9")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 404);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn prediction_returns_probabilities(pool: sqlx::PgPool) {
    PostgresRepository::new(pool.clone())
        .save_race_card(&sample_card())
        .await
        .unwrap();
    let app = build_service!(pool);

    let req = test::TestRequest::get()
        .uri(&format!("/api/races/{RACE_ID}/prediction"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "status: {}", resp.status());
    let json = body_json(resp).await;
    let probs = json["probabilities"].as_array().unwrap();
    assert_eq!(probs.len(), 3);
    // 履歴ゼロでも均等フォールバックで確率が返り、単調性が保たれる。
    for p in probs {
        let win = p["win_prob"].as_f64().unwrap();
        let place = p["place_prob"].as_f64().unwrap();
        let show = p["show_prob"].as_f64().unwrap();
        assert!(win <= place && place <= show, "non-monotonic: {p}");
    }
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn prediction_rejects_out_of_range_blend_alpha(pool: sqlx::PgPool) {
    PostgresRepository::new(pool.clone())
        .save_race_card(&sample_card())
        .await
        .unwrap();
    let app = build_service!(pool);

    let req = test::TestRequest::get()
        .uri(&format!("/api/races/{RACE_ID}/prediction?blend_alpha=2.0"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 400);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn analyze_horse_returns_stats(pool: sqlx::PgPool) {
    let app = build_service!(pool);
    // 履歴が無くても overall を含む統計（ゼロ）が 200 で返る。
    let req = test::TestRequest::get()
        .uri("/api/analyze/horse?name=ウマA")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "status: {}", resp.status());
    let json = body_json(resp).await;
    assert_eq!(json["horse_name"], "ウマA");
    assert!(json["overall"].is_object());
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn extraction_error_returns_error_body(pool: sqlx::PgPool) {
    // クエリ抽出の型変換失敗（f64 パース不能）も、handler 内エラーと同じ ErrorBody 封筒で 400 を返す
    // （app.rs の QueryConfig error_handler 経路）。
    let app = build_service!(pool);
    let req = test::TestRequest::get()
        .uri(&format!("/api/races/{RACE_ID}/prediction?blend_alpha=abc"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 400);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "bad_request");
}
