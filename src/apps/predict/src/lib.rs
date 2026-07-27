//! 予想セッション CLI（`paddock-predict`）の実体。
//!
//! bin.rs から使うだけなら `mod` 宣言で足りるが、統合テスト（`tests/` は別クレートとして
//! コンパイルされる）から `session::run_overview` 等を呼ぶため lib として公開する（#555）。
//! 構成は `src/apps/api-server`（`[lib] api_server` + `[[bin]] paddock-api`）と同型。
pub mod cli;
pub mod session;
pub mod setup;
