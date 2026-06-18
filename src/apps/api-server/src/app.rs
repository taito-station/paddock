use actix_web::dev::Service;
use actix_web::{HttpRequest, web};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use paddock_use_case::pdf_fetcher::PdfFetcher;
use paddock_use_case::pdf_parser::PdfParser;
use paddock_use_case::repository::Repository;
use paddock_use_case::{OddsScraper, PayoutFetcher};
use rest_controller::error::Error as ApiError;
use rest_controller::openapi::ApiDoc;

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
/// ジェネリクス: `R/P/F`=メイン Interactor、`O`=OddsInteractor（odds:refresh）、`S`=SettleInteractor（results:refresh）。
pub fn configure_routes<R, P, F, O, S>(cfg: &mut web::ServiceConfig)
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
    O: OddsScraper + Send + Sync + 'static,
    S: PayoutFetcher + Send + Sync + 'static,
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
            .configure(rest_controller::router::session::configure::<R, P, F, O, S>),
    );

    cfg.service(SwaggerUi::new("/docs/{_:.*}").url("/api-docs/openapi.json", ApiDoc::openapi()));
}
