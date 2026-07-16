# QA: #410 setup.rs のボイラープレート集約

> 質問票+回答（[docs/qa/README.md](README.md)）。一次資料は issue #410（全体レビュー 2026-07-15 由来）。
> 関連: クリーンアーキテクチャ規約 `~/.claude/rules/rust/architecture.md`（依存方向 Apps→Interface→Use-Case→Domain。ADR 0022 / specifications 群と同じ参照形式）。
> 純リファクタ（挙動不変）。集約先がレイヤ違反を作らないことが要件。

## 観測（精密マップ）

- **Noop スタブ重複**: `UnusedParser` / `UnusedFetcher` を **5 apps**（analyze / api-server / odds-collect / predict / predict-watch）の `setup.rs` が各 ~28 行で重複定義。差分はエラーメッセージのアプリ名のみ。`Interactor<R, P: PdfParser, F: PdfFetcher>`（use-case/interactor/mod.rs）が P/F を常時要求するため、PDF を使わない app もスタブを注入している。トレイト `PdfParser`/`PdfFetcher` は **use-case**（pdf_parser.rs / pdf_fetcher.rs）に定義。
- **build_app 重複**: **10 apps** が `Config::from_env → tracing fmt().try_init → pool::connect → pool::migrate` の同一シーケンスを持つ。app 固有差分は scrape delay 等の引数のみ。共通要素（`paddock-config` / `rdb-gateway::pool` / tracing）は全 app が依存済み。

## Q1: Noop スタブの集約先（use-case 提供 か Interactor 分割か）

- 観測/根拠: トレイトは use-case 定義。`Interactor<R,P,F>` は PDF 系ユースケース（ingest_pdf / fetch_meeting）でのみ P/F を使う。
- 回答: **確定（ユーザー決定 2026-07-16）。use-case に `NoopParser` / `NoopFetcher` を提供**（pdf_parser.rs / pdf_fetcher.rs のトレイト近傍）。5 apps の重複を use-case 側 ~10 行に集約し、各 app は `Interactor::new(repo, NoopParser, NoopFetcher)` を呼ぶだけにする。**これはレイヤ違反ではない**（自層トレイトの null-object 実装を自層で提供するだけ）。Interactor のジェネリクス分割は挙動不変要件下で全 impl・全 app DI を触る大改修になり不採用。
- 反映先: `src/use-case/src/pdf_parser.rs` / `pdf_fetcher.rs`、5 apps の setup.rs。

## Q2: build_app 共通シーケンスの集約先（rdb-gateway か新 support crate か）

- 観測/根拠: `pool::connect` / `pool::migrate` は既に **rdb-gateway** の `pool.rs` にある。tracing 初期化は現状各 app が `tracing_subscriber::fmt()` を直に呼ぶ。config 読込は `paddock-config`。
- 回答: **確定（ユーザー決定 2026-07-16）。既存 crate への最小追加**。rdb-gateway の `pool` に `connect_and_migrate(url) -> PgPool` を追加（既存 2 関数の合成・pool 責務内）、tracing 初期化は `paddock-config`（全 app 依存・`paddock_log` を持つ）に `init_tracing(&self)` を生やす（DB 層でない tracing を rdb-gateway に混ぜない）。新 support crate は workspace＋10 app の Cargo.toml churn が大きく不採用。
- 反映先: `src/interface/rdb-gateway/src/pool.rs`、`src/infrastructure/config/`、10 apps の setup.rs。

## Q3: PR 分割（Noop と build_app は独立）

- 観測/根拠: Noop 集約（5 apps）と build_app 集約（10 apps）は互いに独立。1 PR にまとめると diff が大きく（両方で 10 app 超を横断）、レビュー・bisect が重い。
- 回答: **確定（ユーザー決定 2026-07-16）。2 PR に分割**（1 PR 1 トピック）。PR-A: Noop スタブ集約（小・低リスク）→ PR-B: build_app 集約。issue #410 は PR-B（build_app 完了時）で Closes、PR-A は #410 を参照。
- 反映先: PR 運用。
