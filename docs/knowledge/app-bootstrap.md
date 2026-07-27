---
status: Confirmed
kind: knowledge
sources:
  - docs/qa/QA-setup-boilerplate-410.md
  - docs/adr/0069-drop-icloud-writes-browser-only-viewing.md
  - docs/adr/0070-explicit-migration-no-auto-on-startup.md
distilled_from_sha: "8f8be21"
updated: "2026-07-27"
---

# app bootstrap（DI・起動シーケンス）の共通化

新規 app（`src/apps/<bin>`）の `setup.rs` / `bin.rs` を書くときの確定知。#410 で横断的ボイラープレートを既存 crate に集約した（ADR/新規決定は伴わない純リファクタ）。集約先は `~/.claude/rules/rust/architecture.md` の依存方向 Apps→Interface→Use-Case→Domain を崩さない。

## 共通ヘルパ（重複を書かない）

- **接続＋整合チェック**: `rdb_gateway::pool::connect_checked(&config.paddock_db_url, config.paddock_auto_migrate)` を使う（#470/ADR 0070）。起動時 auto-migrate は既定 OFF で、`connect_checked` は read-only 整合チェックのみ（未適用/未初期化なら停止・DB 先行の stale は warn 継続）。prod（compose の `PADDOCK_AUTO_MIGRATE=true`）だけ従来どおり起動時 `migrate` を適用する。マイグレーションの明示適用は `paddock-analyze migrate`。`connect` / `migrate` / `connect_and_migrate` を各 app で個別に呼ばない（pool 責務として rdb-gateway に集約済み）。
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

## PDF 非対応 bin の PdfParser/PdfFetcher ジェネリクス

`Interactor<R, P: PdfParser, F: PdfFetcher>` は P/F を常時要求する。PDF 系ユースケース（`ingest_pdf` / `fetch_meeting`）を呼ばない bin（predict / predict-watch / odds-collect / analyze / api-server 等）は、use-case 共通の **`NoopParser` / `NoopFetcher`**（`paddock_use_case::{NoopParser, NoopFetcher}`）を注入する。各 app で no-op スタブを自前定義しない。誤呼び出し時は `InvalidArgument` で明示失敗する（never-called の dead path）。

## 対象外

- **PDF を実際に扱う bin**（parse-pdf / parse-entries / fetch-card 系）: 本物の `HybridParser` / `MutoolEntryParser` / `JraFetcher` を注入する（Noop ではない）。
