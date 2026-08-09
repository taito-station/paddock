---
status: Confirmed
kind: knowledge
doc_class: [D19, D15]
tags: [D19, D15]
sources:
  - docs/qa/QA-setup-boilerplate-410.md
  - docs/original-docs/0069-drop-icloud-writes-browser-only-viewing.md
  - docs/original-docs/0070-explicit-migration-no-auto-on-startup.md
distilled_from_sha: "3a7e875"
updated: "2026-08-09"
---

# app bootstrap（DI・起動シーケンス）の共通化

新規 app（`src/apps/<bin>`）の `setup.rs` / `bin.rs` を書くときの確定知。#410 で横断的ボイラープレートを既存 crate に集約した（ADR/新規決定は伴わない純リファクタ）。集約先は `~/.claude/rules/rust/architecture.md` の依存方向 Apps→Interface→Use-Case→Domain を崩さない。

## 共通ヘルパ（重複を書かない）

- **接続＋整合チェック**: `rdb_gateway::pool::connect_checked(&config.paddock_db_url, config.paddock_auto_migrate)` を使う（#470/ADR 0070）。起動時 auto-migrate は既定 OFF で、`connect_checked` は read-only 整合チェックのみ（未適用/未初期化なら停止・DB 先行の stale は warn 継続）。適用済み判定は `_sqlx_migrations.success = true` の行だけを見る（**dirty＝前回失敗した行は未適用扱い**）。**pending と stale が同時**（別 worktree が別々に migration を足して交差した状態）なら **Pending を優先して停止**する——自バイナリの未適用 migration があるうちはクエリが壊れうるため。prod（compose の `PADDOCK_AUTO_MIGRATE=true`）だけ従来どおり起動時 `migrate` を適用する。マイグレーションの明示適用は `paddock-analyze migrate`。`connect` / `migrate` / `connect_and_migrate` を各 app で個別に呼ばない（pool 責務として rdb-gateway に集約済み）。
- **tracing 初期化**: `config.init_tracing()` を使う（`paddock_config::Config` のメソッド）。`paddock_log` フィルタで `fmt().with_env_filter(...).try_init()` を実行し、不正フィルタは `info` にフォールバック（#238 の html5ever 抑止の回帰は `default_log_filter_is_valid_env_filter` で担保）。各 app で `tracing_subscriber::fmt()...` を直書きしない。tracing は DB 層の責務でないため rdb-gateway でなく paddock-config（ログ設定 `paddock_log` の持ち主）に置く。

典型的な build_app:

```rust
let config = Config::from_env().context("load config")?;
config.init_tracing();
let pool = pool::connect_checked(&config.paddock_db_url, config.paddock_auto_migrate)
    .await
    .context("connect Postgres")?;
// あとは各 app 固有の Interactor 組み立て（scrape delay 等の差分は引数で吸収）
```

## PDF 系ユースケースは facade ごと分離されている（#453）

- **`Interactor<R>`**（`paddock_use_case::Interactor`）: 非 PDF ユースケース（race / predict / board / live / stats 等）の facade。**Repository のみ**を持つ。
- **`PdfInteractor<R, P: PdfParser, F: PdfFetcher>`**（`paddock_use_case::interactor::pdf`）: PDF 取得・解析を要する `fetch_meeting` / `fetch_meeting_range` / `ingest_pdf` 専用の facade。

したがって **PDF を扱わない bin（predict / predict-watch / odds-collect / analyze / api-server 等）は `Interactor<R>` を組み立てるだけでよく、no-op スタブの注入は要らない**。

> **⚠ 旧記述の訂正**: かつて本節は「P/F を常時要求するので `paddock_use_case::{NoopParser, NoopFetcher}` を注入する」と書いていたが、**#453 で P/F ジェネリクスごと解消され、Noop スタブは削除された**（ソースツリーに存在しない）。この乖離が ADR 0073 の「knowledge を信じると存在しない API を書く」実例（解消は #578）。

PDF 以外にも用途別の facade がある（api-server の DI が実例）。必要なものだけを組み立てる:

- `OddsInteractor<S, R>`: オッズの read-through 取得（#51 / `odds:refresh`）
- `ResultsInteractor<S, R>`: 同日結果の取り込みと自動精算（#381 / `results:refresh`）

## 対象外

- **PDF を実際に扱う bin**: `PdfInteractor` に本物の `HybridParser` / `MutoolEntryParser` / `JraFetcher` を注入する。実運用でこれを構築するのは **`parse-pdf` のみ**。
