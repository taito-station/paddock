//! REST API アダプター（#33, read 基盤）。actix-web の handler / router / schema / error と
//! utoipa による OpenAPI 定義を提供する。Apps 層（api-server）が DI と一緒にマウントする。

pub mod error;
pub mod handler;
pub mod openapi;
pub mod router;
pub mod schema;
