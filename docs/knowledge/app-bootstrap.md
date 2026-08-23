---
status: Confirmed
kind: knowledge
doc_class: [D19, D15]
tags: [D19, D15]
sources:
  - docs/qa/QA-setup-boilerplate-410.md
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

---

## 決定ログ

<!-- この節は append-only です。既存エントリの変更・削除は CI が検出します。 -->

### ADR 0069: iCloud 書き出しを全廃し、閲覧を REST API + SPA に一本化 (2026-07-21) — 承認済み

#### ステータス

承認済み（本 PR で実装）。対象 Issue: [#494](https://github.com/taito-station/paddock/issues/494)。

#### コンテキスト

paddock は macOS の iCloud に 2 系統の書き込みをしていた。予想の閲覧が Obsidian の予想 MD からブラウザ（リッチ版フロント = `api-server` + React SPA）へ移行した結果、この iCloud 運用が不要になった。

- **(A) DB バックアップ iCloud ミラー**（`scripts/backup-db.sh` の `PADDOCK_BACKUP_MIRROR_DIR` 既定 = iCloud Drive `~/Library/Mobile Documents/com~apple~CloudDocs/paddock-backups`）。#494 で顕在化した穴: launchd 下では iCloud への列挙・削除が信頼できず（`cp` は効くが `ls`/`rm` が反映されない macOS file-provider の癖）、世代剪定が恒久 no-op になり dump が無制限に溜まる。ローカル権威 `~/paddock-backups`（`PADDOCK_BACKUP_DIR`）は KEEP=14 で常に剪定が効き、主脅威（Colima volume 喪失）は既にカバー済み。
- **(B) Obsidian/pad MD 書き出し**（`ingest-predictions --render` → iCloud Obsidian vault、唯一の消費者は legacy の `web-viewer`）。予想は DB が正で MD はその生成物（[prediction-json.md](../specifications/prediction-json.md)）。ブラウザ閲覧は DB 直読みの REST API + SPA（ADR 0022）で完結し pad MD に非依存。`--render` は launchd/スクリプトに組み込まれておらず手動実行のみ。

閲覧手段が REST API + SPA に統合された今、iCloud への書き込みは (A)(B) いずれも運用上の価値を失い、(A) は #494 の運用穴として残っていた。

#### 決定

**iCloud への書き込みを全廃し、閲覧は REST API + SPA、バックアップはローカル権威に一本化する。**

1. **DB バックアップの iCloud ミラーを既定 off にする**（#494 解消）。`PADDOCK_BACKUP_MIRROR_DIR` の既定を iCloud パス → **空文字（無効）**に変更。off-machine ミラーは env を明示指定したときだけ有効で、指定先は**実ファイルシステム（外付け/NAS 等）**とする（iCloud は使わない）。ミラー＋剪定コードは汎用処理として残す（既定 off なので通常経路は iCloud に一切触れない）。
2. **Obsidian/pad MD 書き出しパイプラインを廃止する**。`ingest-predictions` から `--render` / `render_all` / `render.rs` / `DEFAULT_PAD_DIR` / 関連 CLI フラグを削除。render 専用の repository メソッド `list_pad_predictions` を trait/gateway から削除。pad MD の唯一の消費者だった `web-viewer` crate（`paddock-web`）を workspace ごと削除。JSON→DB の取り込み（`save_pad_prediction`）は不変。

#### 理由

- **#494 を構造的に解消する**。iCloud を既定ミラー先から外せば「launchd 下で剪定が no-op で溜まる」経路自体が消える。reconcile のリマインダー（issue の代替案）を運用に足すより、原因（iCloud のミラー既定化）を除去する方が「一時的な修正をしない」「課題を後回しにしない」に適う。
- **閲覧は既に REST API + SPA に一本化されている**。web-viewer は DB 非依存で pad MD を読むだけの legacy ビューアであり、pad MD を生成しなくなれば無入力の死コードになる。MD 生成（`--render`）とその唯一の消費者（web-viewer）を同時に畳むのが最小構成。
- **DB が正の原則を崩さない**。予想の永続化は DB（`predict_sessions`/`predict_bets`）で完結しており、MD は派生生成物だった。MD を廃しても予想データと閲覧は損なわれない。

#### 影響

- **削除**: `ingest-predictions` の `--render`/`render.rs`/`DEFAULT_PAD_DIR`／repository `list_pad_predictions`（trait + gateway）／`web-viewer` crate（workspace member と workspace 依存 `pulldown-cmark`）。ドキュメント（README の web-viewer 節・`--render` 記述、`scripts/predict-check/gen_predictions.py` のコメント）。
- **変更（既定値）**: `scripts/backup-db.sh` の `PADDOCK_BACKUP_MIRROR_DIR` 既定を空へ。`deployments/db/BACKUP.md` を「ローカル権威一本、ミラーは非iCloud opt-in」に更新。launchd plist / `install.sh` はミラー系 env を元々注入していないため変更不要。
- **不変**: DB バックアップのローカル権威退避・世代剪定（`~/paddock-backups`・KEEP=14）／`ingest-predictions` の JSON→DB 取り込み／REST API + SPA（`api-server` / `web/`）による閲覧／compose の `web` サービス（React SPA。`paddock-web` という image 名が web-viewer バイナリ名と偶然衝突していたが別物）。
- **トレードオフ**: DB dump の off-machine 冗長（ディスク障害対策）を**既定で失う**。ローカル権威が主脅威（Colima volume 喪失）を外す一方、ディスク障害時は別途 off-machine ミラー（非iCloud パスを env 指定）が必要。実ファイルシステムへの自動ミラー化は将来 issue に委ねる。
- **既存 iCloud 資産の掃除**: 既に iCloud に溜まった DB dump（`.../CloudDocs/paddock-backups`）と予想 MD vault（`.../iCloud~md~obsidian/.../pad`）は手動で削除する（不可逆のためスクリプト化せず運用者が実行）。
- 関連: #265（DB バックアップ）／ADR 0022（REST API read）／#143（web-viewer）／#34（Web SPA）。

### ADR 0070: 起動時 auto-migrate を全廃し、DB マイグレーションを明示適用へ移行 (2026-07-27) — 承認済み

#### ステータス

承認済み（本 PR で実装）。対象 Issue: [#470](https://github.com/taito-station/paddock/issues/470)。

#### コンテキスト

paddock の全 app（predict / api-server / predict-watch / fetch-card / odds-collect / analyze / fetch-history / fetch-results / ingest-predictions / parse-entries / parse-pdf）は起動時に `pool::connect_and_migrate` を呼び、**無条件で** DB マイグレーションを適用していた（#410 で共通化した「接続してからマイグレート」シーケンス）。

この設計は単一 DB を 1 バイナリが占有する前提なら素直だが、paddock は開発運用が異なる。

- **単一 golden DB を複数 worktree/バイナリが共有する**（[compose.yaml](../../deployments/compose.yaml) は worktree ごとに別 database 名で分ける運用を建前として記すが、実運用では seed/回収/バックテストが同一 golden `paddock` DB を叩くため事実上共有される）。同じ golden DB を見るある worktree の新しいバイナリが起動時に自動で DDL を適用すると、別 worktree の古いバイナリから見て DB が先行し、`sqlx` の `VersionMissing`（DB にあるがバイナリが知らない version）で**起動拒否**に至る。
- 起動のたびに無条件で migrate が走ると、「いつ・どの版で DB が進んだか」が起動タイミング依存になり、共有 DB の状態が非決定的になる。

つまり「起動時に全バイナリが無条件で migrate する」ことが、共有 DB モデルと衝突して起動拒否・非決定性を生んでいた。

#### 決定

**起動時の auto-migrate を既定 OFF にし、マイグレーション適用を明示入口に一本化する。**

1. **既定は auto-migrate せず read-only 整合チェックのみ行う**。`pool::connect_checked(url, auto_migrate)` を追加し、全 app の setup を `connect_and_migrate` から差し替える。`auto_migrate=false`（既定）では `pool::check_migration_status` で埋め込み版と DB 適用済み版の差を **DDL を一切発行せず** 調べる（`_sqlx_migrations` を SELECT するだけ。`Migrator::run` / `ensure_migrations_table` を呼ばない）。
2. **非対称 warn ポリシー**で分岐する。
   - `UpToDate` → 何もしない。
   - `StaleBinary`（DB が先行＝このバイナリが古い可能性）→ warn して **継続**。DB が進んでいるだけで当該バイナリの動作は成立しうるため、止めずに「最新ブランチで再ビルドを」と促す。
   - `Pending`（バイナリが知るが DB 未適用）→ warn して **`Err` で停止**。未適用のまま動くと不整合。
   - `Uninitialized`（`_sqlx_migrations` 不在）→ warn して **`Err` で停止**。初回セットアップ未実施。
   - **pending と stale が同時**（別 worktree が別々に migration を足して交差した状態）→ `Pending` を**優先して停止**する。自バイナリの未適用 migration があるうちは（stale であっても）そのバイナリのクエリが壊れうるため動かさず、`paddock-analyze migrate` で自分の分を適用させる。適用済み判定は `_sqlx_migrations.success = true` の行のみ（dirty＝前回失敗した行は未適用扱い）。
3. **明示入口 `paddock-analyze migrate`** を新設する。共有 DB へ未適用マイグレーションを適用する唯一の入口。`--dry-run` で未適用一覧のみ表示する。未初期化 DB でも動く必要があるため、この経路だけは `connect_checked`（Uninitialized で停止する）を経由せず素の `pool::connect` で pool を得る（migrate が自家中毒しない）。
4. **prod は従来どおり起動時 auto-migrate を有効化する**。[compose.yaml](../../deployments/compose.yaml) の `api` / `importer` サービスに `PADDOCK_AUTO_MIGRATE=true` を設定し、コンテナは起動時に自身が `pool::migrate` を適用する（`depends_on` で postgres 健全化を待つ）。Config に `PADDOCK_AUTO_MIGRATE`（既定 `false`）を追加した。

#### 理由

- **共有 golden DB モデルを壊さずに #470 を構造的に解消する**。起動時の無条件 DDL を止めれば、「新バイナリが黙って DB を進め、古いバイナリが `VersionMissing` で起動拒否」という経路自体が消える。
- **StaleBinary で止めないのは実運用の摩擦を避けるため**。DB が先行しているだけなら当該バイナリの機能は概ね成立し、そこで起動拒否すると「古い worktree で作業を続けられない」不便が勝つ。stale は warn で自覚を促すに留め、pending / uninitialized（動くと不整合・初回未セットアップ）だけ止める非対称ポリシーが最小の安全策。
- **明示入口は「課題を後回しにしない」に適う**。migration を追加したら `paddock-analyze migrate` で意図的に共有 DB へ適用する。誰の・いつの起動で DB が進むかが決定的になる。
- **prod は単一デプロイで DB を占有する**ため、起動時 auto-migrate が最も素直。開発の共有 DB とは前提が違うので env で分ける。

#### 却下した代替案

- **Option B: worktree ごとに別 DB を持たせる**。起動時 auto-migrate を維持したまま衝突を避けるには worktree 別 DB にすればよいが、これは **単一 golden DB を共有する** 現行モデル（seed/回収/バックテストが同一 golden を叩く前提）と真っ向から衝突する。golden の複製コスト・鮮度ずれ・回収照合の分断が生じるため却下。共有 DB を保ったまま「起動時に触らない」方向（本決定）を採る。

#### 影響

- **追加**: `pool::MigrationStatus` / `pool::check_migration_status`（read-only）/ `pool::connect_checked`。`rdb_gateway::error::Error::MigrationRequired`（pending / uninitialized の停止用）。`Config::paddock_auto_migrate`（env `PADDOCK_AUTO_MIGRATE`・既定 false）。`paddock-analyze migrate [--dry-run]`。
- **変更**: 全 11 app の setup が `connect_and_migrate` → `connect_checked(url, config.paddock_auto_migrate)`。compose の `api` / `importer` に `PADDOCK_AUTO_MIGRATE=true`。api / importer Dockerfile のコメント。
- **不変**: `pool::connect` / `pool::migrate` / `pool::connect_and_migrate`（温存）。prod の起動時マイグレーション挙動（`PADDOCK_AUTO_MIGRATE=true` で従来同等）。migration ファイル自体（`deployments/db/migrations/`・リバーシブル形式）。
- **運用**: migration 追加後は共有 golden DB へ `paddock-analyze migrate` で明示適用する（起動時には適用されない）。適用は `sqlx` の advisory lock 下で並行安全。stale binary の warn が出たら最新ブランチで再ビルドする。
- 関連: #410（connect/migrate 共通化）／ADR 0069（deployments 周辺運用）。
