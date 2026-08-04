//! `/api/health` が稼働中プロセスの世代（git sha / ビルド時刻）を返すことの検証（#570）。
//!
//! ルート配線自体は `openapi_route_parity.rs` が担保する。ここでは **レスポンス本文**が
//! ビルド時に埋め込んだ `build_info` と一致することを確認し、世代確認の意味論が壊れないよう守る。
//! DB 非依存（health handler は Repository を使わない）なので Postgres 無しで走る。

use actix_web::{App, test as actix_test, web};
use rest_controller::build_info;

#[actix_web::test]
async fn health_reports_build_generation() {
    let app = actix_test::init_service(
        App::new()
            .service(web::scope("/api").configure(rest_controller::router::health::configure)),
    )
    .await;

    let req = actix_test::TestRequest::get()
        .uri("/api/health")
        .to_request();
    let body: serde_json::Value = actix_test::call_and_read_body_json(&app, req).await;

    assert_eq!(body["status"], "ok");
    // ビルド時に埋め込んだ世代情報とレスポンスが一致する（＝現在の checkout と突合可能）。
    assert_eq!(body["git_sha"], build_info::GIT_SHA);
    assert_eq!(
        body["build_time"],
        build_info::build_time_rfc3339().as_str()
    );
    // epoch は常に有効な数値を埋め込むため rfc3339 に整形され、"unknown" にはならない。
    assert_ne!(body["build_time"], "unknown");
}
