use actix_web::web;

use paddock_use_case::pdf_fetcher::PdfFetcher;
use paddock_use_case::pdf_parser::PdfParser;
use paddock_use_case::repository::Repository;

use crate::handler;

/// `/predictions` 配下の read ルートを登録する（呼び出し側が `/api` スコープにマウントする）。
/// 静的セグメント `/stats/by-mark` をパラメータ `/{prediction_id}` より先に登録し、
/// `stats` が ID として誤マッチしないようにする。
pub fn configure<R, P, F>(cfg: &mut web::ServiceConfig)
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    cfg.service(
        web::scope("/predictions")
            .route(
                "",
                web::get().to(handler::prediction::search_predictions::<R, P, F>),
            )
            .route(
                "/stats/by-mark",
                web::get().to(handler::prediction::prediction_mark_stats::<R, P, F>),
            )
            .route(
                "/{prediction_id}",
                web::get().to(handler::prediction::get_prediction_detail::<R, P, F>),
            ),
    );
}
