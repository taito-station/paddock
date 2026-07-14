use actix_web::web;

use paddock_use_case::pdf_fetcher::PdfFetcher;
use paddock_use_case::pdf_parser::PdfParser;
use paddock_use_case::repository::Repository;

use crate::handler;

/// `/analyze` 配下の read ルートを登録する（呼び出し側が `/api` スコープにマウントする）。
pub fn configure<R, P, F>(cfg: &mut web::ServiceConfig)
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    cfg.service(
        web::scope("/analyze")
            .route(
                "/horse",
                web::get().to(handler::analyze::analyze_horse::<R, P, F>),
            )
            .route(
                "/horse/candidates",
                web::get().to(handler::analyze::analyze_horse_candidates::<R, P, F>),
            )
            .route(
                "/jockey",
                web::get().to(handler::analyze::analyze_jockey::<R, P, F>),
            )
            .route(
                "/jockey/candidates",
                web::get().to(handler::analyze::analyze_jockey_candidates::<R, P, F>),
            )
            .route(
                "/trainer",
                web::get().to(handler::analyze::analyze_trainer::<R, P, F>),
            )
            .route(
                "/trainer/candidates",
                web::get().to(handler::analyze::analyze_trainer_candidates::<R, P, F>),
            )
            .route(
                "/course",
                web::get().to(handler::analyze::analyze_course::<R, P, F>),
            ),
    );
}
