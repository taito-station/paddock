//! paddock REST API サーバ（#33, read 基盤）。route 設定（OpenAPI/Swagger UI・認証フック含む）と
//! DI 構築を提供する。`bin.rs` と統合テストの両方から使う。

pub mod app;
pub mod setup;
