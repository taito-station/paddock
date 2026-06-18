use actix_web::dev::Service;
use actix_web::web;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use paddock_use_case::pdf_fetcher::PdfFetcher;
use paddock_use_case::pdf_parser::PdfParser;
use paddock_use_case::repository::Repository;
use rest_controller::openapi::ApiDoc;

/// API の全ルートを登録する（bin・統合テスト共通）。
///
/// - `/api` 配下に read 系ルート（races / analyze）をマウントする。
/// - `/docs` に Swagger UI、`/api-docs/openapi.json` に OpenAPI ドキュメントを配信する。
/// - `/api` スコープに **認証ミドルウェアの差し込み口（現状 no-op）** を 1 箇所だけ用意する。
///   マルチユーザー化の際はこの wrap を JWT 検証に差し替える（`rules/rust/architecture.md` の auth パターン）。
pub fn configure_routes<R, P, F>(cfg: &mut web::ServiceConfig)
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    cfg.service(
        web::scope("/api")
            .wrap_fn(|req, srv| {
                // ===== 認証ミドルウェアの差し込み口（現状 no-op：素通し）=====
                // 将来ここでトークン検証を行い、未認証なら 401 を返す。
                srv.call(req)
            })
            .configure(rest_controller::router::configure::<R, P, F>),
    );

    cfg.service(SwaggerUi::new("/docs/{_:.*}").url("/api-docs/openapi.json", ApiDoc::openapi()));
}
