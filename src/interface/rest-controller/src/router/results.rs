use actix_web::web;

use paddock_use_case::repository::Repository;
use paddock_use_case::result_page_fetcher::ResultPageFetcher;

use crate::handler;

/// `/results` 配下の結果取り込みルートを登録する（呼び出し側が `/api` スコープにマウントする）。
///
/// `S`: `ResultsInteractor<S, R>`（結果ページ取得。`UreqNetkeibaScraper` が `ResultPageFetcher`）。
pub fn configure<S, R>(cfg: &mut web::ServiceConfig)
where
    S: ResultPageFetcher + Send + Sync + 'static,
    R: Repository + 'static,
{
    cfg.service(web::scope("/results").route(
        "/{date}:refresh",
        web::post().to(handler::results::refresh::<S, R>),
    ));
}
