use actix_web::HttpResponse;

use crate::build_info;
use crate::schema::health::HealthResponse;

/// 稼働中プロセスの世代（git sha / ビルド時刻）を返す（#570）。
///
/// 長期稼働した api-server が古い成果物を配信し続けても外形監視（HTTP 200）では気づけない。
/// `git_sha` を現在の checkout と突き合わせることで世代ずれを検知できるようにする。
/// DB 非依存・Repository 非依存なので、DB 未接続でも 200 を返す（liveness プローブも兼ねる）。
#[utoipa::path(
    get,
    path = "/api/health",
    responses(
        (status = 200, description = "稼働中プロセスの世代情報（git sha / ビルド時刻）", body = HealthResponse),
    ),
    tag = "health",
)]
pub async fn health() -> HttpResponse {
    HttpResponse::Ok().json(HealthResponse {
        status: "ok".to_string(),
        git_sha: build_info::GIT_SHA.to_string(),
        build_time: build_info::build_time_rfc3339(),
    })
}
