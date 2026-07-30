use actix_web::web;

use paddock_use_case::repository::Repository;

use crate::handler;

/// `/analyze` 配下の read ルートを登録する（呼び出し側が `/api` スコープにマウントする）。
pub fn configure<R>(cfg: &mut web::ServiceConfig)
where
    R: Repository + 'static,
{
    cfg.service(
        web::scope("/analyze")
            .route(
                "/horse",
                web::get().to(handler::analyze::analyze_horse::<R>),
            )
            .route(
                "/horse/candidates",
                web::get().to(handler::analyze::analyze_horse_candidates::<R>),
            )
            .route(
                "/jockey",
                web::get().to(handler::analyze::analyze_jockey::<R>),
            )
            .route(
                "/jockey/candidates",
                web::get().to(handler::analyze::analyze_jockey_candidates::<R>),
            )
            .route(
                "/trainer",
                web::get().to(handler::analyze::analyze_trainer::<R>),
            )
            .route(
                "/trainer/candidates",
                web::get().to(handler::analyze::analyze_trainer_candidates::<R>),
            )
            .route(
                "/course",
                web::get().to(handler::analyze::analyze_course::<R>),
            ),
    );
}
