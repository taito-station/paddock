pub mod analyze;
pub mod live;
pub mod prediction;
pub mod race;
pub mod results;
pub mod session;

use actix_web::web;

use paddock_use_case::repository::Repository;

/// `/api` 配下に read 系の全ルート（races / analyze）を登録する。
/// 呼び出し側（Apps 層）が `web::scope("/api").configure(rest_controller::router::configure::<R,P,F>)` で使う。
pub fn configure<R>(cfg: &mut web::ServiceConfig)
where
    R: Repository + 'static,
{
    race::configure::<R>(cfg);
    analyze::configure::<R>(cfg);
    prediction::configure::<R>(cfg);
    live::configure::<R>(cfg);
}
