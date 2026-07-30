use actix_web::web;

use paddock_use_case::repository::Repository;

use crate::handler;

/// `/races` 配下の read ルートを登録する（呼び出し側が `/api` スコープにマウントする）。
pub fn configure<R>(cfg: &mut web::ServiceConfig)
where
    R: Repository + 'static,
{
    cfg.service(
        web::scope("/races")
            .route("", web::get().to(handler::race::list_races::<R>))
            .route(
                "/{race_id}",
                web::get().to(handler::race::get_race_card::<R>),
            )
            .route(
                "/{race_id}/prediction",
                web::get().to(handler::race::get_prediction::<R>),
            )
            .route(
                "/{race_id}/recommendations",
                web::get().to(handler::race::get_recommendations::<R>),
            )
            .route(
                "/{race_id}/board",
                web::get().to(handler::race::get_race_board::<R>),
            ),
    );
}
