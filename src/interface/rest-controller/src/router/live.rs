use actix_web::web;

use paddock_use_case::repository::Repository;

use crate::handler;

/// `/live` 配下の read ルートを登録する（呼び出し側が `/api` スコープにマウントする）。
pub fn configure<R>(cfg: &mut web::ServiceConfig)
where
    R: Repository + 'static,
{
    cfg.service(web::scope("/live").route("/{date}", web::get().to(handler::live::get_live::<R>)));
}
