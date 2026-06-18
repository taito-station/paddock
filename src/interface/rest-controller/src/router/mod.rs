pub mod analyze;
pub mod race;

use actix_web::web;

use paddock_use_case::pdf_fetcher::PdfFetcher;
use paddock_use_case::pdf_parser::PdfParser;
use paddock_use_case::repository::Repository;

/// `/api` 配下に read 系の全ルート（races / analyze）を登録する。
/// 呼び出し側（Apps 層）が `web::scope("/api").configure(rest_controller::router::configure::<R,P,F>)` で使う。
pub fn configure<R, P, F>(cfg: &mut web::ServiceConfig)
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    race::configure::<R, P, F>(cfg);
    analyze::configure::<R, P, F>(cfg);
}
