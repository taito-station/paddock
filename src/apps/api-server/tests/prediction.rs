//! 予想横断検索 API（#145）の統合テスト。`#[sqlx::test]` の一時 Postgres を seed し、
//! 各エンドポイントを actix のテストサーバ越しに叩く。
//!
//! 実行には Postgres が必要。`DATABASE_URL=postgres://paddock:paddock@localhost:5432/paddock \
//! cargo test -p api-server --test prediction -- --test-threads=1`。

use actix_web::{App, test, web};
use chrono::{NaiveDate, Utc};
use serde_json::Value;

use api_server::app::configure_routes;
use api_server::setup::{UnusedFetcher, UnusedParser};
use netkeiba_scraper::UreqNetkeibaScraper;
use odds_scraper::UreqOddsScraper;
use paddock_domain::{
    Mark, PadPrediction, PredictionBet, PredictionHorse, PredictionResult, Race, RaceId, Surface,
    Venue,
};
use paddock_use_case::Interactor;
use paddock_use_case::repository::{PadPredictionRepository, RaceRepository};
use rdb_gateway::PostgresRepository;

type Repo = PostgresRepository;

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

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn horse(num: u32, name: &str, mark: Option<Mark>) -> PredictionHorse {
    PredictionHorse {
        horse_num: num,
        horse_name: name.to_string(),
        jockey: None,
        mark,
        win_odds: None,
        popularity: None,
        win_prob: None,
        place_prob: None,
        show_prob: None,
        comment: None,
    }
}

fn result(finish: [Option<u32>; 3], recovery_rate: f64, pnl: i64) -> PredictionResult {
    PredictionResult {
        finish,
        recovery_rate: Some(recovery_rate),
        pnl: Some(pnl),
        note: None,
    }
}

fn prediction(
    d: NaiveDate,
    venue: Venue,
    race_num: u32,
    horses: Vec<PredictionHorse>,
    result: Option<PredictionResult>,
) -> PadPrediction {
    PadPrediction {
        date: d,
        venue,
        race_num,
        title: Some(format!("{venue:?} {race_num}R")),
        budget: Some(10_000),
        strategy_note: None,
        commentary: None,
        horses,
        bets: vec![PredictionBet {
            bet_type: "単勝".to_string(),
            combination: "7".to_string(),
            amount: 600,
        }],
        result,
    }
}

/// 検索・集計テスト共通の seed。
/// - P1: 2026-03-28 中山 11R / ◎ハナミチ(7) ○ウマB(4) / finish=[7,4,13] 回収152% (的中・◎勝ち)
/// - P2: 2026-03-29 阪神 10R / ◎サクラ(3) / finish=[5,3,1] 回収0% (不的中・◎は複勝圏)
/// - P3: 2026-04-05 中山 5R / 無印ノーマーク(1) / 結果なし (pending)
async fn seed(repo: &PostgresRepository) {
    let now = Utc::now();

    // 距離/芝ダ解決用に P1 のレースを seed（date/venue/race_num 一致で race_id 解決）。
    repo.save_race(&Race {
        race_id: RaceId::try_from("2026-3-nakayama-1-R11").unwrap(),
        date: date(2026, 3, 28),
        venue: Venue::Nakayama,
        round: 3,
        day: 1,
        race_num: 11,
        surface: Surface::Turf,
        distance: 2500,
        track_condition: None,
        weather: None,
        results: Vec::new(),
    })
    .await
    .unwrap();

    repo.save_pad_prediction(
        &prediction(
            date(2026, 3, 28),
            Venue::Nakayama,
            11,
            vec![
                horse(7, "ハナミチ", Some(Mark::Honmei)),
                horse(4, "ウマB", Some(Mark::Taikou)),
            ],
            Some(result([Some(7), Some(4), Some(13)], 152.0, 5200)),
        ),
        now,
    )
    .await
    .unwrap();

    repo.save_pad_prediction(
        &prediction(
            date(2026, 3, 29),
            Venue::Hanshin,
            10,
            vec![horse(3, "サクラ", Some(Mark::Honmei))],
            Some(result([Some(5), Some(3), Some(1)], 0.0, -1000)),
        ),
        now,
    )
    .await
    .unwrap();

    repo.save_pad_prediction(
        &prediction(
            date(2026, 4, 5),
            Venue::Nakayama,
            5,
            vec![horse(1, "ノーマーク", None)],
            None,
        ),
        now,
    )
    .await
    .unwrap();
}

/// `GET <uri>` の呼び出し future を返す（呼び出し側で `.await`）。
macro_rules! get {
    ($app:expr, $uri:expr) => {{
        let req = test::TestRequest::get().uri($uri).to_request();
        test::call_service(&$app, req)
    }};
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn search_all_returns_total_and_desc_order(pool: sqlx::PgPool) {
    seed(&PostgresRepository::new(pool.clone())).await;
    let app = build_service!(pool);

    let resp = get!(app, "/api/predictions").await;
    assert!(resp.status().is_success(), "status: {}", resp.status());
    let json = body_json(resp).await;
    assert_eq!(json["total_count"], 3);
    let preds = json["predictions"].as_array().unwrap();
    assert_eq!(preds.len(), 3);
    // date DESC: 2026-04-05 が先頭。
    assert_eq!(preds[0]["date"], "2026-04-05");
    assert_eq!(preds[2]["date"], "2026-03-28");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn search_by_period_and_venue(pool: sqlx::PgPool) {
    seed(&PostgresRepository::new(pool.clone())).await;
    let app = build_service!(pool);

    // 期間で 2026-03 のみ。
    let json = body_json(
        get!(
            app,
            "/api/predictions?date_from=2026-03-01&date_to=2026-03-31"
        )
        .await,
    )
    .await;
    assert_eq!(json["total_count"], 2);

    // 開催場 中山（nakayama）。
    let json = body_json(get!(app, "/api/predictions?venue=nakayama").await).await;
    assert_eq!(json["total_count"], 2);
    let json = body_json(get!(app, "/api/predictions?venue=%E9%98%AA%E7%A5%9E").await).await; // 阪神
    assert_eq!(json["total_count"], 1);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn search_by_distance_surface_excludes_unresolved(pool: sqlx::PgPool) {
    seed(&PostgresRepository::new(pool.clone())).await;
    let app = build_service!(pool);

    // P1 のみ race seed 済み（中山 2500m 芝）。距離/芝ダで絞ると P1 だけ。
    let json = body_json(get!(app, "/api/predictions?surface=turf").await).await;
    assert_eq!(json["total_count"], 1);
    assert_eq!(json["predictions"][0]["distance"], 2500);
    assert_eq!(json["predictions"][0]["surface"], "turf");

    let json =
        body_json(get!(app, "/api/predictions?distance_min=2000&distance_max=3000").await).await;
    assert_eq!(json["total_count"], 1);

    // 範囲外は 0 件（未照合の P2/P3 も distance フィルタで脱落）。
    let json = body_json(get!(app, "/api/predictions?distance_min=3000").await).await;
    assert_eq!(json["total_count"], 0);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn search_by_horse_name_partial(pool: sqlx::PgPool) {
    seed(&PostgresRepository::new(pool.clone())).await;
    let app = build_service!(pool);

    // "ハナ" 部分一致 → P1 のみ。%E3... は "ハナ" の percent-encode。
    let json = body_json(get!(app, "/api/predictions?horse_name=%E3%83%8F%E3%83%8A").await).await;
    assert_eq!(json["total_count"], 1);
    assert_eq!(json["predictions"][0]["honmei_horse"], "ハナミチ");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn search_by_mark_and_hit(pool: sqlx::PgPool) {
    seed(&PostgresRepository::new(pool.clone())).await;
    let app = build_service!(pool);

    // 印 honmei を含む予想は P1/P2（P3 は無印）。
    let json = body_json(get!(app, "/api/predictions?mark=honmei").await).await;
    assert_eq!(json["total_count"], 2);

    // 的中 (recovery_rate>0) は P1 のみ。
    let json = body_json(get!(app, "/api/predictions?hit=true").await).await;
    assert_eq!(json["total_count"], 1);
    assert_eq!(json["predictions"][0]["hit"], true);

    // 不的中 (結果あり且つ払戻0) は P2 のみ（P3 は結果未記録で対象外）。
    let json = body_json(get!(app, "/api/predictions?hit=false").await).await;
    assert_eq!(json["total_count"], 1);
    assert_eq!(json["predictions"][0]["hit"], false);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn search_pagination(pool: sqlx::PgPool) {
    seed(&PostgresRepository::new(pool.clone())).await;
    let app = build_service!(pool);

    let json = body_json(get!(app, "/api/predictions?limit=1&offset=1").await).await;
    assert_eq!(json["total_count"], 3); // 総件数はフィルタ適用後の全件
    assert_eq!(json["limit"], 1);
    assert_eq!(json["offset"], 1);
    assert_eq!(json["predictions"].as_array().unwrap().len(), 1);
    // date DESC の 2 件目 = 2026-03-29。
    assert_eq!(json["predictions"][0]["date"], "2026-03-29");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn detail_found_and_not_found(pool: sqlx::PgPool) {
    seed(&PostgresRepository::new(pool.clone())).await;
    let app = build_service!(pool);

    // 一覧から prediction_id を取り、個別取得。
    let json = body_json(get!(app, "/api/predictions?venue=hanshin").await).await;
    let id = json["predictions"][0]["prediction_id"].as_i64().unwrap();

    let json = body_json(get!(app, &format!("/api/predictions/{id}")).await).await;
    assert_eq!(json["prediction_id"], id);
    assert_eq!(json["venue"], "hanshin");
    assert_eq!(json["horses"].as_array().unwrap().len(), 1);
    assert_eq!(json["horses"][0]["mark"], "honmei");
    assert_eq!(json["bets"].as_array().unwrap().len(), 1);
    assert_eq!(json["result"]["finish"], serde_json::json!([5, 3, 1]));

    // 未存在 id → 404。
    let resp = get!(app, "/api/predictions/999999").await;
    assert_eq!(resp.status().as_u16(), 404);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "not_found");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn mark_stats_by_mark(pool: sqlx::PgPool) {
    seed(&PostgresRepository::new(pool.clone())).await;
    let app = build_service!(pool);

    let json = body_json(get!(app, "/api/predictions/stats/by-mark").await).await;
    let by_mark = json["by_mark"].as_array().unwrap();
    // 結果記録済みは P1(◎7,○4) と P2(◎3)。◎ count=2(P1勝ち+P2複勝), ○ count=1。
    let honmei = by_mark.iter().find(|m| m["mark"] == "honmei").unwrap();
    assert_eq!(honmei["count"], 2);
    assert_eq!(honmei["win"], 1); // P1 の◎(7)は1着、P2 の◎(3)は2着
    assert_eq!(honmei["show"], 2); // 両方とも複勝圏
    assert!((honmei["win_rate"].as_f64().unwrap() - 0.5).abs() < 1e-9);

    let taikou = by_mark.iter().find(|m| m["mark"] == "taikou").unwrap();
    assert_eq!(taikou["count"], 1);
    assert_eq!(taikou["win"], 0); // P1 の○(4)は2着
    assert_eq!(taikou["show"], 1);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn rejects_bad_params(pool: sqlx::PgPool) {
    seed(&PostgresRepository::new(pool.clone())).await;
    let app = build_service!(pool);

    for uri in [
        "/api/predictions?surface=grass",        // 不正 surface
        "/api/predictions?mark=bogus",           // 不正 mark
        "/api/predictions?date_from=2026-13-01", // 不正日付
        "/api/predictions?date_from=2026-04-01&date_to=2026-03-01", // 逆転期間
        "/api/predictions?distance_min=3000&distance_max=2000", // 逆転距離
        "/api/predictions?venue=foo",            // 不正 venue
        "/api/predictions/stats/by-mark?date_from=bad", // 集計の不正日付
    ] {
        let resp = get!(app, uri).await;
        assert_eq!(resp.status().as_u16(), 400, "uri={uri}");
        let json = body_json(resp).await;
        assert_eq!(json["error"]["code"], "bad_request", "uri={uri}");
    }
}
