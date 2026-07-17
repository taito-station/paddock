//! REST API（read, #33）の統合テスト。`#[sqlx::test]` の一時 Postgres DB を seed し、
//! 各エンドポイントを actix のテストサーバ越しに叩く。
//!
//! 実行には Postgres が必要（`#[sqlx::test]` が一時 DB を作る）。`cargo test -p api-server`。
//! Postgres 非接続環境ではコンパイルのみ確認できる（`cargo test --no-run`）。

use actix_web::{App, test, web};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;

use api_server::app::configure_routes;
use netkeiba_scraper::UreqNetkeibaScraper;
use paddock_domain::{
    FinishingPosition, GateNum, HorseEntry, HorseName, HorseNum, HorseResult, JockeyName, Race,
    RaceCard, RaceId, RacePayouts, ResultStatus, Surface, Venue,
};
use paddock_use_case::netkeiba_scraper::ResultRow;
use paddock_use_case::repository::{
    OddsRepository, OddsRow, PredictBetRecord, PredictSessionRecord, PredictSessionRepository,
    RaceCardRepository, RaceOddsRecord, RaceRepository, RaceResultRepository,
};
use paddock_use_case::result_page_fetcher::ResultPageFetcher;
use paddock_use_case::{Interactor, NoopFetcher, NoopParser, ResultsInteractor};
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
        post_time: None,
        venue: Venue::Nakayama,
        round: 1,
        day: 1,
        race_num: 1,
        surface: Surface::Turf,
        distance: 1800,
        race_class: Some(paddock_domain::RaceClass::G3),
        // レース名（#389）。card / board / list レスポンスに載ることを検証する。
        race_name: Some("サンプルステークス".to_string()),
        entries: vec![
            entry(1, 1, "ウマA"),
            entry(2, 2, "ウマB"),
            entry(3, 3, "ウマC"),
        ],
    }
}

/// 部分一致候補（#401）検証用に `results` へ 1 頭分の成績を積む Race。horse/jockey は正規化を通る。
fn result_race(race_id: &str, horse: &str, jockey: &str) -> Race {
    Race {
        race_id: RaceId::try_from(race_id).unwrap(),
        date: date(),
        venue: Venue::Tokyo,
        round: 3,
        day: 2,
        race_num: 1,
        surface: Surface::Turf,
        distance: 2000,
        track_condition: None,
        weather: None,
        results: vec![HorseResult {
            finishing_position: Some(FinishingPosition::try_from(1u32).unwrap()),
            status: ResultStatus::Finished,
            gate_num: GateNum::try_from(1u32).unwrap(),
            horse_num: HorseNum::try_from(1u32).unwrap(),
            horse_name: HorseName::try_from(horse).unwrap(),
            horse_id: None,
            jockey: Some(JockeyName::try_from(jockey).unwrap()),
            trainer: None,
            time_seconds: None,
            margin: None,
            odds: None,
            horse_weight: None,
            weight_change: None,
            weight_carried: None,
            popularity: None,
        }],
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
        let interactor = Interactor::new(repo, NoopParser, NoopFetcher);
        let data = web::Data::new(interactor);
        test::init_service(App::new().app_data(data).configure(
            configure_routes::<
                Repo,
                NoopParser,
                NoopFetcher,
                UreqNetkeibaScraper,
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
    // 出馬表（race_cards）未保存なら post_time は null（#391）。
    assert!(races[0]["post_time"].is_null());
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn list_races_includes_post_time_from_race_cards(pool: sqlx::PgPool) {
    // #391: 発走時刻は race_cards を一次ソースに HH:MM で返す（watch 判定記録に依存しない）。
    let repo = PostgresRepository::new(pool.clone());
    repo.save_race(&sample_race()).await.unwrap();
    let mut card = sample_card();
    card.post_time = Some(chrono::NaiveTime::from_hms_opt(15, 45, 0).unwrap());
    repo.save_race_card(&card).await.unwrap();
    let app = build_service!(pool);

    let req = test::TestRequest::get()
        .uri("/api/races?date=2026-03-28")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "status: {}", resp.status());

    let json = body_json(resp).await;
    let races = json["races"].as_array().unwrap();
    assert_eq!(races.len(), 1);
    assert_eq!(races[0]["post_time"], "15:45");
    // レース名も race_cards 由来で一覧に載る（#389）。
    assert_eq!(races[0]["race_name"], "サンプルステークス");
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
    // レース名・格付けが出馬表レスポンスに載る（#389）。
    assert_eq!(json["race_name"], "サンプルステークス");
    assert_eq!(json["race_class"], "g3");

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
/// #272 循環断ち以降、recommendations の EV/的中は純モデル(α=1.0)×市場odds で計算され blend_alpha に
/// 依らない（blend_alpha は軸/相手の順位付けにのみ効く）。本フィクスチャでは順位も一致するため、
/// 素モデル(1.0)と省略時(0.2)で買い目は完全一致する＝EV が blend 不変であることの回帰。
/// （ブレンドが確率に作用すること自体は予測エンドポイントの
/// `prediction_omitted_blend_alpha_equals_production_default` が担保する。）
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

    // blend_alpha=1.0（素モデル）でも、EV/的中は純モデル固定なので省略時(0.2)と同じ EV になる。
    // このフィクスチャでは順位（軸/相手）も一致するため、買い目（組合せ・stake・EV）は完全一致する。
    // = recommendations の EV が blend_alpha に依存しない（循環断ち, #272）ことの回帰。
    let req_raw = test::TestRequest::get()
        .uri(&format!(
            "/api/races/{RACE_ID}/recommendations?budget=10000&blend_alpha=1.0"
        ))
        .to_request();
    let json_raw = body_json(test::call_service(&app, req_raw).await).await;
    assert_eq!(
        json_omit["bets"], json_raw["bets"],
        "EV は純モデル固定（#272）。本フィクスチャでは順位も一致し買い目は blend_alpha に依らず同一"
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
async fn board_without_odds_returns_all_horses(pool: sqlx::PgPool) {
    PostgresRepository::new(pool.clone())
        .save_race_card(&sample_card())
        .await
        .unwrap();
    let app = build_service!(pool);

    let req = test::TestRequest::get()
        .uri(&format!("/api/races/{RACE_ID}/board"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "status: {}", resp.status());
    let json = body_json(resp).await;

    // 全頭が truncate されず返る（出馬表 3 頭）。
    let horses = json["horses"].as_array().unwrap();
    assert_eq!(horses.len(), 3);
    assert_eq!(json["field_size"], 3);
    // オッズ未 seed → 買い目なし・市場implied/人気は null だが確率と model_rank は出る。
    assert_eq!(json["odds_available"], false);
    assert_eq!(json["bets"].as_array().unwrap().len(), 0);
    for h in horses {
        assert!(h["market_implied"].is_null());
        assert!(h["popularity"].is_null());
        assert!(h["model_rank"].as_u64().unwrap() >= 1);
    }
    // 混戦サマリは常に返る。
    assert!(json["confusion"].is_object());
    assert!(json["confusion"]["is_confused"].is_boolean());
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn board_with_odds_returns_market_and_bets(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool.clone());
    repo.save_race_card(&sample_card()).await.unwrap();
    repo.save_race_odds(&sample_win_odds()).await.unwrap();
    repo.save_race_odds(&sample_odds()).await.unwrap();
    let app = build_service!(pool);

    let req = test::TestRequest::get()
        .uri(&format!("/api/races/{RACE_ID}/board"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "status: {}", resp.status());
    let json = body_json(resp).await;

    assert_eq!(json["horses"].as_array().unwrap().len(), 3);
    assert_eq!(json["odds_available"], true);
    // 単勝 seed 済み → 市場implied・人気が全頭に付く。
    for h in json["horses"].as_array().unwrap() {
        assert!(h["market_implied"].is_number(), "implied: {h}");
        assert!(h["popularity"].as_u64().unwrap() >= 1);
    }
    // 買い目は /recommendations と同経路（相手 top5 不変）→ 非空。
    assert!(!json["bets"].as_array().unwrap().is_empty());
    assert!(json["roi"].is_number());
}

/// board の買い目（axis/partners/bets/roi）は /recommendations と完全一致する（同経路の担保）。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn board_bets_match_recommendations(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool.clone());
    repo.save_race_card(&sample_card()).await.unwrap();
    repo.save_race_odds(&sample_win_odds()).await.unwrap();
    repo.save_race_odds(&sample_odds()).await.unwrap();
    let app = build_service!(pool);

    let board = body_json(
        test::call_service(
            &app,
            test::TestRequest::get()
                .uri(&format!("/api/races/{RACE_ID}/board?budget=10000"))
                .to_request(),
        )
        .await,
    )
    .await;
    let reco = body_json(
        test::call_service(
            &app,
            test::TestRequest::get()
                .uri(&format!(
                    "/api/races/{RACE_ID}/recommendations?budget=10000"
                ))
                .to_request(),
        )
        .await,
    )
    .await;

    assert_eq!(board["axis"], reco["axis"], "軸が一致");
    assert_eq!(board["partners"], reco["partners"], "相手が一致");
    assert_eq!(
        board["bets"], reco["bets"],
        "買い目が一致（相手 top5 不変）"
    );
    assert_eq!(board["roi"], reco["roi"], "ROI が一致");

    // 盤の ◎（model_rank==1 の馬）＝買い目軸（build_portfolio の axis）が一致することを固定する。
    // 両者は blended 首位・同一 tie-break（win_prob 降順→馬番昇順）由来なのでズレない前提を回帰で担保。
    let axis = board["axis"].as_u64().unwrap();
    let top = board["horses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["model_rank"].as_u64() == Some(1))
        .expect("model_rank==1 の馬が存在する");
    assert_eq!(
        top["horse_num"].as_u64().unwrap(),
        axis,
        "盤の ◎(model_rank 1) と買い目軸は同一馬でなければならない"
    );
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
async fn analyze_candidates_partial_match(pool: sqlx::PgPool) {
    // results に "ダイワ" を含む 2 頭 + 騎手を seed（#401）。ルート結線・q 正規化・{names,truncated} 形を検証。
    let repo = PostgresRepository::new(pool.clone());
    repo.save_race(&result_race(
        "2026-3-tokyo-2-1R",
        "ダイワスカーレット",
        "ルメール",
    ))
    .await
    .unwrap();
    repo.save_race(&result_race(
        "2026-3-tokyo-2-2R",
        "ダイワメジャー",
        "横山和生",
    ))
    .await
    .unwrap();

    let app = build_service!(pool);

    // "ダイワ"（部分）→ 名前昇順で 2 件・truncated=false。q の非 ASCII は percent-encode する。
    let req = test::TestRequest::get()
        .uri("/api/analyze/horse/candidates?q=%E3%83%80%E3%82%A4%E3%83%AF")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "status: {}", resp.status());
    let json = body_json(resp).await;
    assert_eq!(
        json["names"],
        serde_json::json!(["ダイワスカーレット", "ダイワメジャー"])
    );
    assert_eq!(json["truncated"], false);

    // 騎手も同経路で候補が引ける（/jockey/candidates のルート結線確認）。"ルメ" → ["ルメール"]。
    let jreq = test::TestRequest::get()
        .uri("/api/analyze/jockey/candidates?q=%E3%83%AB%E3%83%A1")
        .to_request();
    let jresp = test::call_service(&app, jreq).await;
    assert!(jresp.status().is_success(), "status: {}", jresp.status());
    let jjson = body_json(jresp).await;
    assert_eq!(jjson["names"], serde_json::json!(["ルメール"]));

    // trainer/candidates の結線確認 + 0 件（該当なし）の封筒。seed に trainer は無いので
    // 任意語でも空。3 ハンドラは同型なので trainer は結線と空応答の代表として最小検証する
    // （truncated=true は handler ユニット over_limit_is_truncated で担保）。"ゾゾ" = 該当なし。
    let treq = test::TestRequest::get()
        .uri("/api/analyze/trainer/candidates?q=%E3%82%BE%E3%82%BE")
        .to_request();
    let tresp = test::call_service(&app, treq).await;
    assert!(tresp.status().is_success(), "status: {}", tresp.status());
    let tjson = body_json(tresp).await;
    assert_eq!(tjson["names"], serde_json::json!([]));
    assert_eq!(tjson["truncated"], false);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn live_summary_includes_server_now(pool: sqlx::PgPool) {
    // snapshot 無しの日でも 200（races 空）。summary.server_now が rfc3339 で載ることを検証（#382）。
    let app = build_service!(pool);
    let before = Utc::now();
    let req = test::TestRequest::get()
        .uri("/api/live/2026-07-11")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "status: {}", resp.status());
    let json = body_json(resp).await;
    assert!(json["races"].as_array().unwrap().is_empty());
    let server_now = json["summary"]["server_now"]
        .as_str()
        .expect("server_now は文字列");
    let parsed = DateTime::parse_from_rfc3339(server_now)
        .unwrap_or_else(|e| panic!("server_now が rfc3339 でない: {server_now} ({e})"));
    // レスポンス生成時刻はリクエスト前後の妥当な範囲に収まる（秒精度なので下限は 1 秒緩める）。
    assert!(
        parsed.with_timezone(&Utc) >= before - chrono::Duration::seconds(1),
        "server_now {server_now} が before {before} より前"
    );
    assert!(
        parsed.with_timezone(&Utc) <= Utc::now(),
        "server_now {server_now} が現在より未来"
    );
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

// --- #381 レース結果の同日取り込み・API 公開 -----------------------------------

/// 着順付き Race（`results` を seed）。finishing_position 1..=3 を horse_num 1..=3 に割り当てる。
fn sample_race_with_results() -> Race {
    let hr = |pos: u32, num: u32, name: &str| HorseResult {
        finishing_position: Some(FinishingPosition::try_from(pos).unwrap()),
        status: ResultStatus::Finished,
        gate_num: GateNum::try_from(num).unwrap(),
        horse_num: HorseNum::try_from(num).unwrap(),
        horse_name: HorseName::try_from(name).unwrap(),
        horse_id: None,
        jockey: None,
        trainer: None,
        time_seconds: None,
        margin: None,
        odds: None,
        horse_weight: None,
        weight_change: None,
        weight_carried: None,
        popularity: None,
    };
    Race {
        results: vec![hr(1, 1, "ウマA"), hr(2, 2, "ウマB"), hr(3, 3, "ウマC")],
        ..sample_race()
    }
}

/// 結果ページ取得のフェイク（着順 rows ＋ 確定払戻を canned で返す）。id は無視する（テストは 1 レース）。
struct FakeResultPage {
    rows: Vec<ResultRow>,
    payouts: RacePayouts,
}

impl ResultPageFetcher for FakeResultPage {
    fn fetch_race_result_page(
        &self,
        _netkeiba_race_id: &str,
    ) -> paddock_use_case::Result<(Vec<ResultRow>, RacePayouts)> {
        Ok((self.rows.clone(), self.payouts.clone()))
    }
}

/// refresh 系テスト用 race_id。`netkeiba_race_id_from_paddock` が変換できる形式（末尾 `{num}R`）。
/// （読み取り系テストの `RACE_ID`＝`...-R1` は RaceId としては有効だが netkeiba 変換不可のため別立て。）
const RESULTS_RACE_ID: &str = "2026-4-tokyo-1-1R";

/// refresh 系テスト用の出馬表（`RESULTS_RACE_ID`・出走 3 頭）。`post_time` は引数で差し替える。
fn results_card(post_time: Option<chrono::NaiveTime>) -> RaceCard {
    RaceCard {
        race_id: RaceId::try_from(RESULTS_RACE_ID).unwrap(),
        date: date(),
        post_time,
        venue: Venue::Tokyo,
        round: 4,
        day: 1,
        race_num: 1,
        surface: Surface::Turf,
        distance: 1600,
        race_class: None,
        race_name: None,
        entries: vec![
            entry(1, 1, "ウマA"),
            entry(2, 2, "ウマB"),
            entry(3, 3, "ウマC"),
        ],
    }
}

fn result_row(num: u32, pos: u32) -> ResultRow {
    ResultRow {
        horse_num: HorseNum::try_from(num).unwrap(),
        finishing_position: Some(FinishingPosition::try_from(pos).unwrap()),
        status: ResultStatus::Finished,
        jockey: None,
        trainer: None,
        time_seconds: None,
        odds: None,
        horse_weight: None,
        weight_change: None,
        weight_carried: None,
        popularity: None,
    }
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn list_races_exposes_result_confirmed_and_finish_order(pool: sqlx::PgPool) {
    // #381: 着順が入っていれば result_confirmed=true・finish_order に上位着順が出る。
    PostgresRepository::new(pool.clone())
        .save_race(&sample_race_with_results())
        .await
        .unwrap();
    let app = build_service!(pool);

    let req = test::TestRequest::get()
        .uri("/api/races?date=2026-03-28")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "status: {}", resp.status());
    let json = body_json(resp).await;
    let race = &json["races"][0];
    assert_eq!(race["result_confirmed"], true);
    let finish = race["finish_order"].as_array().unwrap();
    assert_eq!(finish.len(), 3);
    assert_eq!(finish[0]["position"], 1);
    assert_eq!(finish[0]["horse_num"], 1);
    assert_eq!(finish[0]["horse_name"], "ウマA");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn list_races_result_confirmed_false_without_results(pool: sqlx::PgPool) {
    // 着順が無いレース（出馬表のみ）は result_confirmed=false・finish_order 空。
    PostgresRepository::new(pool.clone())
        .save_race_card(&sample_card())
        .await
        .unwrap();
    let app = build_service!(pool);

    let req = test::TestRequest::get()
        .uri("/api/races?date=2026-03-28")
        .to_request();
    let resp = test::call_service(&app, req).await;
    let json = body_json(resp).await;
    let race = &json["races"][0];
    assert_eq!(race["result_confirmed"], false);
    assert_eq!(race["finish_order"].as_array().unwrap().len(), 0);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn board_exposes_finishing_position_and_result_confirmed(pool: sqlx::PgPool) {
    // #381: 盤の各馬に確定着順・盤に result_confirmed が出る（出馬表＋着順を seed）。
    let repo = PostgresRepository::new(pool.clone());
    repo.save_race_card(&sample_card()).await.unwrap();
    repo.save_race(&sample_race_with_results()).await.unwrap();
    let app = build_service!(pool);

    let req = test::TestRequest::get()
        .uri(&format!("/api/races/{RACE_ID}/board"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "status: {}", resp.status());
    let json = body_json(resp).await;
    assert_eq!(json["result_confirmed"], true);
    let horses = json["horses"].as_array().unwrap();
    // 各馬に finishing_position が付く（horse_num == finishing_position で seed）。
    for h in horses {
        let num = h["horse_num"].as_u64().unwrap();
        assert_eq!(
            h["finishing_position"].as_u64().unwrap(),
            num,
            "horse {num} finishing_position"
        );
    }
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn results_interactor_ingests_and_settles(pool: sqlx::PgPool) {
    // #381: 同日取り込み＋自動精算のエンドツーエンド（実 PostgresRepository ＋ フェイク結果取得）。
    let repo = PostgresRepository::new(pool.clone());
    // 出馬表（発走時刻は過去日 2026-03-28 の 10:00 → 発走済み扱い）。
    repo.save_race_card(&results_card(Some(
        chrono::NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
    )))
    .await
    .unwrap();
    // セッション＋単勝①の買い目（stake 1000）。
    let race_id = RaceId::try_from(RESULTS_RACE_ID).unwrap();
    let session = PredictSessionRecord {
        date: date(),
        budget: 10000,
        balance: 9000,
        total_bet: 1000,
        total_payout: 0,
        completed: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    repo.save_predict_session(&session).await.unwrap();
    let bets = vec![PredictBetRecord {
        race_id: race_id.clone(),
        bet_type: "win".to_string(),
        combination: "1".to_string(),
        stake: 1000,
        payout: 0,
        ev: 1.5,
    }];
    repo.save_race_outcome(&session, &race_id, &bets)
        .await
        .unwrap();

    // フェイク: ①が1着、単勝①の払戻 250（＝2.5倍）。
    let rows = vec![result_row(1, 1), result_row(2, 2), result_row(3, 3)];
    let mut payouts = RacePayouts::empty(race_id.clone());
    payouts.insert("win", "1", 250);
    let interactor = ResultsInteractor::new(
        FakeResultPage { rows, payouts },
        PostgresRepository::new(pool.clone()),
    );

    let report = interactor.refresh(date(), false).await.unwrap();
    assert_eq!(report.newly_confirmed_races, 1);
    assert_eq!(report.settled_races, 1);
    assert_eq!(report.pending_races, 0);
    // 単勝 1000/100*250 = 2500。
    assert_eq!(report.total_payout, 2500);
    assert_eq!(report.balance, 10000 - 1000 + 2500);

    // 着順が results に入り、確定フラグが立つ。
    let confirmed = repo.find_result_confirmed_by_date(date()).await.unwrap();
    assert_eq!(confirmed.get(&race_id), Some(&true));
    let positions = repo.find_finishing_positions(&race_id).await.unwrap();
    assert_eq!(positions.get(&1), Some(&1));

    // 冪等: 2 回目は確定済みで netkeiba を叩かず、集計は同値・新規確定 0。
    let report2 = interactor.refresh(date(), false).await.unwrap();
    assert_eq!(report2.newly_confirmed_races, 0);
    assert_eq!(report2.settled_races, 1, "AlreadySettled は settled に算入");
    assert_eq!(report2.total_payout, 2500);
    assert_eq!(report2.balance, 10000 - 1000 + 2500);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn results_interactor_skips_post_time_missing_without_force(pool: sqlx::PgPool) {
    // post_time 未取得は「発走済みと断定しない」（#391）→ force=false で対象外、force=true で救済取り込み。
    let repo = PostgresRepository::new(pool.clone());
    repo.save_race_card(&results_card(None)).await.unwrap();
    let race_id = RaceId::try_from(RESULTS_RACE_ID).unwrap();
    let rows = vec![result_row(1, 1), result_row(2, 2), result_row(3, 3)];
    let payouts = RacePayouts::empty(race_id.clone());
    let interactor = ResultsInteractor::new(
        FakeResultPage { rows, payouts },
        PostgresRepository::new(pool.clone()),
    );

    // force=false: post_time 欠損は対象外 → 未確定のまま。
    let report = interactor.refresh(date(), false).await.unwrap();
    assert_eq!(report.newly_confirmed_races, 0);
    assert!(
        !repo
            .find_result_confirmed_by_date(date())
            .await
            .unwrap()
            .contains_key(&race_id)
    );

    // force=true: gating 緩和で取り込む。
    let report2 = interactor.refresh(date(), true).await.unwrap();
    assert_eq!(report2.newly_confirmed_races, 1);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn results_refresh_endpoint_routes_and_ingests(pool: sqlx::PgPool) {
    // HTTP レベルで `POST /api/results/{date}:refresh`（混在セグメント）の経路解決＋レスポンスを検証。
    // フェイク結果取得を注入した ResultsInteractor を app_data に載せる。
    let repo = PostgresRepository::new(pool.clone());
    repo.save_race_card(&results_card(Some(
        chrono::NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
    )))
    .await
    .unwrap();

    let main = web::Data::new(Interactor::new(
        PostgresRepository::new(pool.clone()),
        NoopParser,
        NoopFetcher,
    ));
    let rows = vec![result_row(1, 1), result_row(2, 2), result_row(3, 3)];
    let payouts = RacePayouts::empty(RaceId::try_from(RESULTS_RACE_ID).unwrap());
    let results = web::Data::new(ResultsInteractor::new(
        FakeResultPage { rows, payouts },
        PostgresRepository::new(pool.clone()),
    ));
    let app = test::init_service(App::new().app_data(main).app_data(results).configure(
        configure_routes::<Repo, NoopParser, NoopFetcher, UreqNetkeibaScraper, FakeResultPage>,
    ))
    .await;

    // 新エンドポイント（既定 force=false・post_time 過去で発走済み）。
    let req = test::TestRequest::post()
        .uri("/api/results/2026-03-28:refresh")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "status: {}", resp.status());
    let json = body_json(resp).await;
    assert_eq!(json["newly_confirmed_races"], 1);
    assert_eq!(json["confirmed_race_ids"][0], RESULTS_RACE_ID);

    // エイリアス（/sessions/{date}/results:refresh・force=true 委譲）も経路解決する。
    let req2 = test::TestRequest::post()
        .uri("/api/sessions/2026-03-28/results:refresh")
        .to_request();
    let resp2 = test::call_service(&app, req2).await;
    assert!(
        resp2.status().is_success(),
        "alias status: {}",
        resp2.status()
    );
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn upsert_results_preserves_races_meta_and_absent_rows(pool: sqlx::PgPool) {
    // #381 回帰防止: 同日 upsert は save_race と違い races メタ（track_condition/weather）を
    // NULL 上書きせず、出馬表に無い馬番の既存着順行も消さない（過去日 refresh でのデータ欠損防止）。
    let repo = PostgresRepository::new(pool.clone());
    let race_id = RaceId::try_from(RESULTS_RACE_ID).unwrap();
    let hr = |pos: u32, num: u32, name: &str| HorseResult {
        finishing_position: Some(FinishingPosition::try_from(pos).unwrap()),
        status: ResultStatus::Finished,
        gate_num: GateNum::try_from(num).unwrap(),
        horse_num: HorseNum::try_from(num).unwrap(),
        horse_name: HorseName::try_from(name).unwrap(),
        horse_id: None,
        jockey: None,
        trainer: None,
        time_seconds: None,
        margin: None,
        odds: None,
        horse_weight: None,
        weight_change: None,
        weight_carried: None,
        popularity: None,
    };
    // 既存: PDF 由来相当の races 行（3 頭の着順）。
    let existing = Race {
        race_id: race_id.clone(),
        date: date(),
        venue: Venue::Tokyo,
        round: 4,
        day: 1,
        race_num: 1,
        surface: Surface::Turf,
        distance: 1600,
        track_condition: None,
        weather: None,
        results: vec![hr(1, 1, "ウマA"), hr(2, 2, "ウマB"), hr(3, 3, "ウマC")],
    };
    repo.save_race(&existing).await.unwrap();
    // 馬場・天候を後付け（結果ページには載る一方 ResultRow には無い列＝温存対象）。
    sqlx::query("UPDATE races SET track_condition = 'good', weather = 'sunny' WHERE race_id = $1")
        .bind(RESULTS_RACE_ID)
        .execute(&pool)
        .await
        .unwrap();

    // 同日 upsert を 1 頭分だけで実行（他 2 頭は「今回集合に無い」）。
    repo.upsert_results(&results_card(None), &[result_row(1, 1)])
        .await
        .unwrap();

    // races メタが温存される（NULL 上書きされない）。
    let (tc, wx): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT track_condition, weather FROM races WHERE race_id = $1")
            .bind(RESULTS_RACE_ID)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(tc.as_deref(), Some("good"), "track_condition 温存");
    assert_eq!(wx.as_deref(), Some("sunny"), "weather 温存");

    // 出馬表に無い馬番の既存着順行が消えない（delete_absent なし）→ 3 頭のまま。
    let positions = repo.find_finishing_positions(&race_id).await.unwrap();
    assert_eq!(positions.len(), 3, "既存着順が削除されない");
}
