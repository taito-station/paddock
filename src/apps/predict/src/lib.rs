//! 予想セッション CLI（`paddock-predict`）の実体。
//!
//! bin.rs から使うだけなら `mod` 宣言で足りるが、統合テスト（`tests/` は別クレートとして
//! コンパイルされる）から `session::run_overview` 等を呼ぶため lib として公開する（#555）。
//! 構成は `src/apps/api-server`（`[lib] api_server` + `[[bin]] paddock-api`）と同型。
//!
//! ここで公開している item は bin と自クレートの統合テストのための内部公開であり、
//! 外部クレート向けのサポート対象 API ではない（後方互換を保証しない）。
//! lib 化により `pub` item は `dead_code` 検知の対象外になるので、bin / 統合テストが
//! 実際に必要としない item は `pub(crate)` に留めて公開面を膨らませないこと。
pub mod cli;
pub mod session;
pub mod setup;
