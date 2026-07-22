use actix_web::dev::Service;
use actix_web::{HttpRequest, web};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use paddock_use_case::OddsScraper;
use paddock_use_case::pdf_fetcher::PdfFetcher;
use paddock_use_case::pdf_parser::PdfParser;
use paddock_use_case::repository::Repository;
use paddock_use_case::result_page_fetcher::ResultPageFetcher;
use rest_controller::error::Error as ApiError;
use rest_controller::openapi::ApiDoc;

/// `configure_routes` が実際に登録する API ルートの «正典リスト»（`(HTTP メソッド, パス)`）。
///
/// ルート登録は各 `router/*.rs` の `web::scope` に分散しており、actix-web 4 には登録済みルートを
/// 実行時に列挙する公開 API が無い。そこで「実ルート ↔ `ApiDoc` の paths」の突合（#457）は、この
/// 単一の定数を介して行う:
///
/// - `tests/openapi_route_parity.rs` が **本リストと `ApiDoc::openapi().paths` を両方向で突合**し、
///   「handler を配線したが `openapi.rs` の `paths(...)` に列挙し忘れた」欠落／その逆を fail 検知する。
/// - 同テストが **本リストの各ルートへ実リクエストを投げ 404 でないこと（＝実際に登録済み）** も確認し、
///   このリストが実配線から乖離して嘘をつくのを防ぐ。
///
/// ルートを追加・変更したら、`router/*.rs`（実配線）・`openapi.rs`（`paths(...)`）・本リストの 3 箇所を
/// 揃える。1 つでも欠ければ上記テストが落ちる。パスは `openapi.rs` の `#[utoipa::path(path = ...)]` と
/// 同じ `/api` プレフィックス付き・`{param}` 表記で書く（突合時に正規化不要）。
///
/// 限界: actix-web 4 の制約上、実配線を機械列挙できないため、「router に配線したが本リストにも
/// `paths(...)` にも書かなかった」完全な列挙漏れは検知できない（3 箇所すべてで漏らした場合）。本リストと
/// `paths(...)` のどちらか一方にでも現れれば突合で検知する。
pub const REGISTERED_ROUTES: &[(&str, &str)] = &[
    // races（read）
    ("GET", "/api/races"),
    ("GET", "/api/races/{race_id}"),
    ("GET", "/api/races/{race_id}/prediction"),
    ("GET", "/api/races/{race_id}/recommendations"),
    ("GET", "/api/races/{race_id}/board"),
    // analyze（read）
    ("GET", "/api/analyze/horse"),
    ("GET", "/api/analyze/horse/candidates"),
    ("GET", "/api/analyze/jockey"),
    ("GET", "/api/analyze/jockey/candidates"),
    ("GET", "/api/analyze/trainer"),
    ("GET", "/api/analyze/trainer/candidates"),
    ("GET", "/api/analyze/course"),
    // predictions（read）
    ("GET", "/api/predictions"),
    ("GET", "/api/predictions/stats/by-mark"),
    ("GET", "/api/predictions/{prediction_id}"),
    // live（read）
    ("GET", "/api/live/{date}"),
    // sessions（write）
    ("POST", "/api/sessions/{date}"),
    ("GET", "/api/sessions/{date}"),
    ("POST", "/api/sessions/{date}/races/{race_id}/outcome"),
    ("POST", "/api/sessions/{date}/races/{race_id}/odds:refresh"),
    ("POST", "/api/sessions/{date}/results:refresh"),
    // results（write, #381）
    ("POST", "/api/results/{date}:refresh"),
];

/// クエリ/パス/JSON ボディの抽出失敗（型変換・必須欠落・不正 JSON）も、handler 内エラーと同じ
/// `{ "error": { code, message } }` 封筒で返すための共通ハンドラ。
fn bad_request<E: std::fmt::Display>(err: E, _req: &HttpRequest) -> actix_web::Error {
    ApiError::BadRequest(err.to_string()).into()
}

/// API の全ルートを登録する（bin・統合テスト共通）。
///
/// - `/api` 配下に read 系（races / analyze）と session write 系をマウントする。
/// - `/docs` に Swagger UI、`/api-docs/openapi.json` に OpenAPI ドキュメントを配信する。
/// - `/api` スコープに **認証ミドルウェアの差し込み口（現状 no-op）** を 1 箇所だけ用意する。
///   マルチユーザー化の際はこの wrap を JWT 検証に差し替える（`rules/rust/architecture.md` の auth パターン）。
///   Swagger UI / openapi.json は `/api` の外に置いており、現状この認証フックの対象外
///   （将来 docs を保護したい場合は配置・wrap を見直す）。
///
/// ジェネリクス: `R/P/F`=メイン Interactor、`O`=OddsInteractor（odds:refresh）、`S`=ResultsInteractor（results:refresh, #381）。
pub fn configure_routes<R, P, F, O, S>(cfg: &mut web::ServiceConfig)
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
    O: OddsScraper + Send + Sync + 'static,
    S: ResultPageFetcher + Send + Sync + 'static,
{
    // 抽出エラーを ErrorBody 封筒へ正規化する（actix 既定のプレーンテキスト 400 を避ける）。
    cfg.app_data(web::QueryConfig::default().error_handler(bad_request));
    cfg.app_data(web::PathConfig::default().error_handler(bad_request));
    cfg.app_data(web::JsonConfig::default().error_handler(bad_request));

    cfg.service(
        web::scope("/api")
            .wrap_fn(|req, srv| {
                // ===== 認証ミドルウェアの差し込み口（現状 no-op：素通し）=====
                // 将来ここでトークン検証を行い、未認証なら 401 を返す。
                srv.call(req)
            })
            .configure(rest_controller::router::configure::<R, P, F>)
            .configure(rest_controller::router::results::configure::<S, R>)
            .configure(rest_controller::router::session::configure::<R, P, F, O, S>),
    );

    cfg.service(SwaggerUi::new("/docs/{_:.*}").url("/api-docs/openapi.json", ApiDoc::openapi()));
}
