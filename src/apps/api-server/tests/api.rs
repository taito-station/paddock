//! REST API（read, #33）の統合テスト。`#[sqlx::test]` の一時 Postgres DB を seed し、
//! 各エンドポイントを actix のテストサーバ越しに叩く。
//!
//! 実行には Postgres が必要（`#[sqlx::test]` が一時 DB を作る）。`cargo test -p api-server`。
//! Postgres 非接続環境ではコンパイルのみ確認できる（`cargo test --no-run`）。

use actix_web::{App, test, web};
use chrono::{NaiveDate, Utc};
use serde_json::Value;

use api_server::app::configure_routes;
use api_server::setup::{UnusedFetcher, UnusedParser};
use netkeiba_scraper::UreqNetkeibaScraper;
use odds_scraper::UreqOddsScraper;
use paddock_domain::{
    GateNum, HorseEntry, HorseName, HorseNum, Race, RaceCard, RaceId, Surface, Venue,
};
use paddock_use_case::Interactor;
use paddock_use_case::repository::{
    OddsRepository, OddsRow, RaceCardRepository, RaceOddsRecord, RaceRepository,
};
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

/// blend 検証用の単勝オッズ（horse_num 1〜3）。[`RACE_ID`] と同じレースに紐づける。
fn sample_win_odds() -> RaceOddsRecord {
    let row = |key: &str, odds: f64| OddsRow {
        bet_type: "win".to_string(),
        combination_key: key.to_string(),
        odds,
        odds_high: None,
        popularity: None,
    };
    RaceOddsRecord {
        race_id: RaceId::try_from(RACE_ID).unwrap(),
        fetched_at: Utc::now(),
        rows: vec![row("1", 2.5), row("2", 4.0), row("3", 6.0)],
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
    // blend_alpha 省略時は PRODUCTION_BLEND_ALPHA=0.2 が渡るが、オッズ未 seed のため
    // ブレンドはスキップされ素モデルで動作する（predict.rs の no-odds フォールバック）。
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

/// blend_alpha 省略時は `PRODUCTION_BLEND_ALPHA`(0.2) が適用され、明示した 0.2 と同一結果を返す。
/// 単勝オッズを seed してブレンドが実際に走る状態で比較する（no-odds 時は両者とも素モデルで差異なし）。
/// 加えて素モデル(blend_alpha=1.0)と差が出ることも確認し、ブレンドが実際に作用していることを保証する。
/// `sample_win_odds` は 3 頭に odds 2.5/4.0/6.0 を与え、均等 prior を崩すためブレンドで差異が生じる。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn prediction_omitted_blend_alpha_equals_production_default(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool.clone());
    repo.save_race_card(&sample_card()).await.unwrap();
    repo.save_race_odds(&sample_win_odds()).await.unwrap();
    let app = build_service!(pool);

    let req_omit = test::TestRequest::get()
        .uri(&format!("/api/races/{RACE_ID}/prediction"))
        .to_request();
    let json_omit = body_json(test::call_service(&app, req_omit).await).await;

    // 0.2 は PRODUCTION_BLEND_ALPHA と同値
    let req_explicit = test::TestRequest::get()
        .uri(&format!("/api/races/{RACE_ID}/prediction?blend_alpha=0.2"))
        .to_request();
    let json_explicit = body_json(test::call_service(&app, req_explicit).await).await;

    assert_eq!(
        json_omit["probabilities"], json_explicit["probabilities"],
        "省略時と明示 0.2 の確率は一致する"
    );

    // blend_alpha=1.0 は素モデル（オッズ不使用）→ ブレンドが実際に作用していれば結果が異なる
    let req_raw = test::TestRequest::get()
        .uri(&format!("/api/races/{RACE_ID}/prediction?blend_alpha=1.0"))
        .to_request();
    let json_raw = body_json(test::call_service(&app, req_raw).await).await;
    assert_ne!(
        json_omit["probabilities"], json_raw["probabilities"],
        "省略時（ブレンド）と素モデル(1.0)は異なる確率を返す"
    );
}

/// recommendations も blend_alpha 省略時は `PRODUCTION_BLEND_ALPHA`(0.2) が適用され、明示した 0.2 と同一結果を返す。
/// 素モデル(blend_alpha=1.0)との差異でブレンドが実際に作用していることを保証する。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn recommendations_omitted_blend_alpha_equals_production_default(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool.clone());
    repo.save_race_card(&sample_card()).await.unwrap();
    repo.save_race_odds(&sample_win_odds()).await.unwrap();
    repo.save_race_odds(&sample_odds()).await.unwrap();
    let app = build_service!(pool);

    let req_omit = test::TestRequest::get()
        .uri(&format!(
            "/api/races/{RACE_ID}/recommendations?budget=10000"
        ))
        .to_request();
    let json_omit = body_json(test::call_service(&app, req_omit).await).await;

    // 0.2 は PRODUCTION_BLEND_ALPHA と同値
    let req_explicit = test::TestRequest::get()
        .uri(&format!(
            "/api/races/{RACE_ID}/recommendations?budget=10000&blend_alpha=0.2"
        ))
        .to_request();
    let json_explicit = body_json(test::call_service(&app, req_explicit).await).await;

    assert_eq!(
        json_omit["bets"], json_explicit["bets"],
        "省略時と明示 0.2 の買い目は一致する"
    );

    // blend_alpha=1.0 は素モデル（オッズ不使用）→ ブレンドが実際に作用していれば買い目が異なる
    // 馬番の組み合わせが同じでも確率が変わると各脚への stake 配分が変わるため bets 全体が差異を持つ
    let req_raw = test::TestRequest::get()
        .uri(&format!(
            "/api/races/{RACE_ID}/recommendations?budget=10000&blend_alpha=1.0"
        ))
        .to_request();
    let json_raw = body_json(test::call_service(&app, req_raw).await).await;
    assert_ne!(
        json_omit["bets"], json_raw["bets"],
        "省略時（ブレンド）と素モデル(1.0)は異なる買い目を返す"
    );
}

/// 保存オッズ（quinella/wide/trio）を 1 レース分 seed する。axis=1, partners=2,3 を流す想定。
fn sample_odds() -> RaceOddsRecord {
    let row = |bet_type: &str, key: &str, odds: f64, odds_high: Option<f64>| OddsRow {
        bet_type: bet_type.to_string(),
        combination_key: key.to_string(),
        odds,
        odds_high,
        popularity: None,
    };
    RaceOddsRecord {
        race_id: RaceId::try_from(RACE_ID).unwrap(),
        fetched_at: Utc::now(),
        rows: vec![
            row("quinella", "1-2", 5.0, None),
            row("quinella", "1-3", 8.0, None),
            row("wide", "1-2", 2.0, Some(3.0)),
            row("wide", "1-3", 3.0, Some(4.5)),
            row("trio", "1-2-3", 25.0, None),
        ],
    }
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn recommendations_without_saved_odds_is_empty(pool: sqlx::PgPool) {
    PostgresRepository::new(pool.clone())
        .save_race_card(&sample_card())
        .await
        .unwrap();
    let app = build_service!(pool);

    let req = test::TestRequest::get()
        .uri(&format!(
            "/api/races/{RACE_ID}/recommendations?budget=10000"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "status: {}", resp.status());
    let json = body_json(resp).await;
    assert_eq!(json["odds_available"], false);
    assert_eq!(json["bets"].as_array().unwrap().len(), 0);
    assert_eq!(json["total_stake"], 0);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn recommendations_with_saved_odds_returns_portfolio(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool.clone());
    repo.save_race_card(&sample_card()).await.unwrap();
    repo.save_race_odds(&sample_odds()).await.unwrap();
    let app = build_service!(pool);

    let req = test::TestRequest::get()
        .uri(&format!(
            "/api/races/{RACE_ID}/recommendations?budget=10000"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "status: {}", resp.status());
    let json = body_json(resp).await;
    assert_eq!(json["odds_available"], true);
    let bets = json["bets"].as_array().unwrap();
    assert!(!bets.is_empty(), "保存オッズがあれば買い目が出る");
    // 予算内・100 円単位で収まる。
    let total = json["total_stake"].as_u64().unwrap();
    assert!(total <= 10000, "total_stake {total} > budget");
    assert_eq!(total % 100, 0, "100 円単位");
    // 券種ラベル・組合せ・EV など from_portfolio の写経フィールドが揃っていることを確認する。
    for b in bets {
        let t = b["bet_type"].as_str().unwrap();
        assert!(
            matches!(t, "馬連" | "ワイド" | "三連複"),
            "想定外の券種: {t}"
        );
        // 組合せキーは a-b / a-b-c 形式（to_key 由来）。
        assert!(b["combination"].as_str().unwrap().contains('-'));
        assert!(b["stake"].as_u64().is_some());
        assert!(b["ev"].is_number(), "ev フィールドが伝播している");
    }
    // seed は全脚に odds を持ち、3 頭立てなので simulate による回収率・的中率が算出される。
    assert!(
        json["roi"].is_number(),
        "オッズ付き脚があるので roi 非 null"
    );
    assert!(
        json["hit_prob"].is_number(),
        "オッズ付き脚があるので hit_prob 非 null"
    );
    // 少なくとも 1 脚は保存オッズが乗る（odds 非 null）。
    assert!(
        bets.iter().any(|b| b["odds"].is_number()),
        "保存済みオッズが脚に反映される"
    );
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn recommendations_rejects_out_of_range_blend_alpha(pool: sqlx::PgPool) {
    let app = build_service!(pool);
    // クエリ検証は DB アクセス前に走るので seed 不要。
    let req = test::TestRequest::get()
        .uri(&format!(
            "/api/races/{RACE_ID}/recommendations?budget=10000&blend_alpha=2.0"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 400);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "bad_request");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn recommendations_rejects_zero_budget(pool: sqlx::PgPool) {
    PostgresRepository::new(pool.clone())
        .save_race_card(&sample_card())
        .await
        .unwrap();
    let app = build_service!(pool);

    let req = test::TestRequest::get()
        .uri(&format!("/api/races/{RACE_ID}/recommendations?budget=0"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 400);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "bad_request");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn analyze_horse_returns_stats(pool: sqlx::PgPool) {
    let app = build_service!(pool);
    // 履歴が無くても overall を含む統計（ゼロ）が 200 で返る。
    // クエリ値の非 ASCII は実 HTTP クライアント同様に percent-encode する（生のマルチバイトは
    // URI として不正で http::Uri パースが InvalidUriChar で弾く）。"ウマA" の UTF-8 = %E3%82%A6%E3%83%9EA。
    let req = test::TestRequest::get()
        .uri("/api/analyze/horse?name=%E3%82%A6%E3%83%9EA")
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
