use actix_web::web;

use paddock_use_case::pdf_fetcher::PdfFetcher;
use paddock_use_case::pdf_parser::PdfParser;
use paddock_use_case::repository::Repository;

use crate::handler;

/// `/live` 配下の read ルートを登録する（呼び出し側が `/api` スコープにマウントする）。
pub fn configure<R, P, F>(cfg: &mut web::ServiceConfig)
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    cfg.service(
        web::scope("/live").route("/{date}", web::get().to(handler::live::get_live::<R, P, F>)),
    );
}
