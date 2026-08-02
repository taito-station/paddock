use actix_web::web;

use crate::handler;

/// `/health` ルートを登録する（呼び出し側が `/api` スコープにマウントする → `/api/health`）。
/// Repository 非依存なのでジェネリクスを持たない。
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.route("/health", web::get().to(handler::health::health));
}
