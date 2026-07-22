//! 実ルート ↔ `ApiDoc` の paths 突合テスト（#457）。
//!
//! `tests/openapi.rs` のスナップショットは「生成結果も期待値も同じ `ApiDoc` 由来」で自己参照のため、
//! **handler を配線したが `openapi.rs` の `paths(...)` へ列挙し忘れた**欠落を検知できない。ここでは
//! 実ルート集合（`app::REGISTERED_ROUTES`）と `ApiDoc::openapi().paths` を **両方向で突合**し、
//! さらに正典リストの各ルートが **実際に登録済み（実リクエストで 404 でない）** ことを確認する。
//!
//! Postgres 不要: ルート解決（path+method マッチ）は DB アクセス前に済むため、遅延接続プールで足りる。
//! 詳細な設計と限界は `app::REGISTERED_ROUTES` の doc を参照。

use std::collections::BTreeSet;

use actix_web::{App, test as actix_test, web};
use sqlx::postgres::PgPoolOptions;
use utoipa::OpenApi;

use api_server::app::{REGISTERED_ROUTES, configure_routes};
use netkeiba_scraper::UreqNetkeibaScraper;
use paddock_use_case::result_page_fetcher::ResultPageFetcher;
use paddock_use_case::{Interactor, NoopFetcher, NoopParser, ResultsInteractor};
use rdb_gateway::PostgresRepository;
use rest_controller::openapi::ApiDoc;

type Repo = PostgresRepository;

/// `ApiDoc::openapi().paths` を `(METHOD, path)` 集合に展開する。
/// `PathItem` はメソッドごとに `Option<Operation>` を持つので、存在するものだけ拾う。
fn apidoc_routes() -> BTreeSet<(String, String)> {
    let openapi = ApiDoc::openapi();
    let mut set = BTreeSet::new();
    for (path, item) in openapi.paths.paths {
        let methods = [
            ("GET", item.get.is_some()),
            ("PUT", item.put.is_some()),
            ("POST", item.post.is_some()),
            ("DELETE", item.delete.is_some()),
            ("OPTIONS", item.options.is_some()),
            ("HEAD", item.head.is_some()),
            ("PATCH", item.patch.is_some()),
            ("TRACE", item.trace.is_some()),
        ];
        for (method, present) in methods {
            if present {
                set.insert((method.to_string(), path.clone()));
            }
        }
    }
    set
}

/// `REGISTERED_ROUTES` を `(METHOD, path)` 集合に正規化する（メソッドは大文字化のみ）。
fn registered_routes() -> BTreeSet<(String, String)> {
    REGISTERED_ROUTES
        .iter()
        .map(|(m, p)| (m.to_uppercase(), p.to_string()))
        .collect()
}

/// `ApiDoc.paths` と `REGISTERED_ROUTES` が **両方向で一致**することを検証する（#457 の主眼）。
///
/// - 実配線あり × `paths(...)` 無し → 「handler を配線したが OpenAPI 列挙を忘れた」欠落を検知。
/// - `paths(...)` あり × 実配線無し → 「paths に書いたが router へ配線し忘れた／消した」を検知。
#[test]
fn registered_routes_match_openapi_paths() {
    let doc = apidoc_routes();
    let registered = registered_routes();

    let missing_in_doc: Vec<_> = registered.difference(&doc).collect();
    let missing_in_routes: Vec<_> = doc.difference(&registered).collect();

    assert!(
        missing_in_doc.is_empty(),
        "実ルート（REGISTERED_ROUTES）にあるが ApiDoc の paths(...) に無い（列挙漏れ）: {missing_in_doc:?}\n\
         → src/interface/rest-controller/src/openapi.rs の paths(...) に handler を追加してください。"
    );
    assert!(
        missing_in_routes.is_empty(),
        "ApiDoc の paths(...) にあるが実ルート（REGISTERED_ROUTES）に無い（配線漏れ/リスト漏れ）: {missing_in_routes:?}\n\
         → router への配線と src/apps/api-server/src/app.rs の REGISTERED_ROUTES を確認してください。"
    );
}

/// パステンプレートの各 `{param}` を経路解決可能な具体値に置換する。
/// 値の妥当性は問わない（404 でなければ「登録済み」と判定できるため）。
fn concrete_uri(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            // '}' まで読み飛ばし、プレースホルダ名で具体値を決める。
            let mut name = String::new();
            for c in chars.by_ref() {
                if c == '}' {
                    break;
                }
                name.push(c);
            }
            let value = match name.as_str() {
                "date" => "2026-01-01",
                _ => "x",
            };
            out.push_str(value);
        } else {
            out.push(c);
        }
    }
    out
}

/// `(app, method, uri)` へ実リクエストし、**ルートが解決されたか** を bool で返す。
///
/// actix の `web::scope` は「path がどのルートにもマッチしない」場合に **本文空の 404** を返す。
/// 一方 handler が返す 404（`Error::NotFound`）は `{ "error": { "code": "not_found", ... } }` の
/// JSON 封筒を持つ。したがって「解決された」= **404 以外**、または **本文が当 API のエラー封筒を持つ 404**。
/// これにより DB の有無（handler が 404 NotFound か 500 Internal のどちらに倒れるか）に依らず判定できる。
/// （App の具体型は `init_service` 由来で名前が長いため、型注釈を避けてマクロで展開する。）
macro_rules! route_is_wired {
    ($app:expr, $method:expr, $uri:expr) => {{
        let req = match ($method as &str).to_uppercase().as_str() {
            "GET" => actix_test::TestRequest::get(),
            "POST" => actix_test::TestRequest::post(),
            "PUT" => actix_test::TestRequest::put(),
            "DELETE" => actix_test::TestRequest::delete(),
            "PATCH" => actix_test::TestRequest::patch(),
            other => panic!("未対応の HTTP メソッド {other}（テストに追加してください）"),
        }
        .uri($uri)
        .to_request();
        let resp = actix_test::call_service($app, req).await;
        if resp.status().as_u16() != 404 {
            true
        } else {
            // 404 の場合のみ本文を見る: 当 API のエラー封筒なら handler が返した 404＝ルート解決済み。
            let body = actix_test::read_body(resp).await;
            serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.get("code")).cloned())
                .is_some()
        }
    }};
}

/// `REGISTERED_ROUTES` の各ルートが **実際に登録済み**（テスト App へ実リクエストして解決される）で
/// あることを確認する。正典リストが実配線から乖離して嘘をつくのを防ぐガード。
///
/// 併せて負のコントロールとして、明らかに未登録なパスが「解決されない」と判定されることも確認し、
/// 判定ロジック（`route_is_wired!`）自体が常に true を返す壊れ方をしていないことを担保する。
#[actix_web::test]
async fn every_registered_route_is_wired() {
    // 遅延接続プール（この時点では接続しない）。DB が無ければ handler が 500（Internal）に倒れるが、
    // それも「ルートは解決された」証拠になる（`route_is_wired!` 参照）。
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://paddock:paddock@127.0.0.1:5432/paddock")
        .expect("build lazy pool");
    let interactor = web::Data::new(Interactor::new(
        PostgresRepository::new(pool.clone()),
        NoopParser,
        NoopFetcher,
    ));
    let results = web::Data::new(ResultsInteractor::new(
        NoopResultPage,
        PostgresRepository::new(pool.clone()),
    ));
    let app =
        actix_test::init_service(App::new().app_data(interactor).app_data(results).configure(
            configure_routes::<Repo, NoopParser, NoopFetcher, UreqNetkeibaScraper, NoopResultPage>,
        ))
        .await;

    // 負のコントロール: 未登録パスは解決されない（本文空 404）。
    assert!(
        !route_is_wired!(&app, "GET", "/api/__definitely_not_registered__"),
        "判定ロジックの健全性: 未登録パスは wired と判定されてはならない"
    );

    for (method, path) in REGISTERED_ROUTES {
        let uri = concrete_uri(path);
        assert!(
            route_is_wired!(&app, method, &uri),
            "{method} {path}（→ {uri}）が未解決: REGISTERED_ROUTES に載っているが実際には登録されていない。\n\
             router への配線と正典リストの表記（/api プレフィックス・{{param}} 表記）を確認してください。",
        );
    }
}

/// 結果ページ取得のダミー実装（`ResultsInteractor` を app_data に載せるためだけに使う）。
/// ルート解決の検証では handler 本体まで到達しないので、呼ばれない前提の no-op で足りる。
struct NoopResultPage;

impl ResultPageFetcher for NoopResultPage {
    async fn fetch_race_result_page(
        &self,
        _netkeiba_race_id: &str,
    ) -> paddock_use_case::Result<(
        Vec<paddock_use_case::netkeiba_scraper::ResultRow>,
        paddock_domain::RacePayouts,
    )> {
        unreachable!("ルート解決テストは handler 本体へ到達しない")
    }
}
