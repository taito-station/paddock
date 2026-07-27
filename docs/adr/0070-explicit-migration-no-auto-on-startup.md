# 0070. 起動時 auto-migrate を全廃し、DB マイグレーションを明示適用へ移行

## ステータス

承認済み（本 PR で実装）。対象 Issue: [#470](https://github.com/taito-station/paddock/issues/470)。

## コンテキスト

paddock の全 app（predict / api-server / predict-watch / fetch-card / odds-collect / analyze / fetch-history / fetch-results / ingest-predictions / parse-entries / parse-pdf）は起動時に `pool::connect_and_migrate` を呼び、**無条件で** DB マイグレーションを適用していた（#410 で共通化した「接続してからマイグレート」シーケンス）。

この設計は単一 DB を 1 バイナリが占有する前提なら素直だが、paddock は開発運用が異なる。

- **単一 golden DB を複数 worktree/バイナリが共有する**（[compose.yaml](../../deployments/compose.yaml) は worktree ごとに別 database 名で分ける運用を建前として記すが、実運用では seed/回収/バックテストが同一 golden `paddock` DB を叩くため事実上共有される）。同じ golden DB を見るある worktree の新しいバイナリが起動時に自動で DDL を適用すると、別 worktree の古いバイナリから見て DB が先行し、`sqlx` の `VersionMissing`（DB にあるがバイナリが知らない version）で**起動拒否**に至る。
- 起動のたびに無条件で migrate が走ると、「いつ・どの版で DB が進んだか」が起動タイミング依存になり、共有 DB の状態が非決定的になる。

つまり「起動時に全バイナリが無条件で migrate する」ことが、共有 DB モデルと衝突して起動拒否・非決定性を生んでいた。

## 決定

**起動時の auto-migrate を既定 OFF にし、マイグレーション適用を明示入口に一本化する。**

1. **既定は auto-migrate せず read-only 整合チェックのみ行う**。`pool::connect_checked(url, auto_migrate)` を追加し、全 app の setup を `connect_and_migrate` から差し替える。`auto_migrate=false`（既定）では `pool::check_migration_status` で埋め込み版と DB 適用済み版の差を **DDL を一切発行せず** 調べる（`_sqlx_migrations` を SELECT するだけ。`Migrator::run` / `ensure_migrations_table` を呼ばない）。
2. **非対称 warn ポリシー**で分岐する。
   - `UpToDate` → 何もしない。
   - `StaleBinary`（DB が先行＝このバイナリが古い可能性）→ warn して **継続**。DB が進んでいるだけで当該バイナリの動作は成立しうるため、止めずに「最新ブランチで再ビルドを」と促す。
   - `Pending`（バイナリが知るが DB 未適用）→ warn して **`Err` で停止**。未適用のまま動くと不整合。
   - `Uninitialized`（`_sqlx_migrations` 不在）→ warn して **`Err` で停止**。初回セットアップ未実施。
3. **明示入口 `paddock-analyze migrate`** を新設する。共有 DB へ未適用マイグレーションを適用する唯一の入口。`--dry-run` で未適用一覧のみ表示する。未初期化 DB でも動く必要があるため、この経路だけは `connect_checked`（Uninitialized で停止する）を経由せず素の `pool::connect` で pool を得る（migrate が自家中毒しない）。
4. **prod は従来どおり起動時 auto-migrate を有効化する**。[compose.yaml](../../deployments/compose.yaml) の `api` / `importer` サービスに `PADDOCK_AUTO_MIGRATE=true` を設定し、コンテナは起動時に自身が `pool::migrate` を適用する（`depends_on` で postgres 健全化を待つ）。Config に `PADDOCK_AUTO_MIGRATE`（既定 `false`）を追加した。

## 理由

- **共有 golden DB モデルを壊さずに #470 を構造的に解消する**。起動時の無条件 DDL を止めれば、「新バイナリが黙って DB を進め、古いバイナリが `VersionMissing` で起動拒否」という経路自体が消える。
- **StaleBinary で止めないのは実運用の摩擦を避けるため**。DB が先行しているだけなら当該バイナリの機能は概ね成立し、そこで起動拒否すると「古い worktree で作業を続けられない」不便が勝つ。stale は warn で自覚を促すに留め、pending / uninitialized（動くと不整合・初回未セットアップ）だけ止める非対称ポリシーが最小の安全策。
- **明示入口は「課題を後回しにしない」に適う**。migration を追加したら `paddock-analyze migrate` で意図的に共有 DB へ適用する。誰の・いつの起動で DB が進むかが決定的になる。
- **prod は単一デプロイで DB を占有する**ため、起動時 auto-migrate が最も素直。開発の共有 DB とは前提が違うので env で分ける。

## 却下した代替案

- **Option B: worktree ごとに別 DB を持たせる**。起動時 auto-migrate を維持したまま衝突を避けるには worktree 別 DB にすればよいが、これは **単一 golden DB を共有する** 現行モデル（seed/回収/バックテストが同一 golden を叩く前提）と真っ向から衝突する。golden の複製コスト・鮮度ずれ・回収照合の分断が生じるため却下。共有 DB を保ったまま「起動時に触らない」方向（本決定）を採る。

## 影響

- **追加**: `pool::MigrationStatus` / `pool::check_migration_status`（read-only）/ `pool::connect_checked`。`rdb_gateway::error::Error::MigrationRequired`（pending / uninitialized の停止用）。`Config::paddock_auto_migrate`（env `PADDOCK_AUTO_MIGRATE`・既定 false）。`paddock-analyze migrate [--dry-run]`。
- **変更**: 全 11 app の setup が `connect_and_migrate` → `connect_checked(url, config.paddock_auto_migrate)`。compose の `api` / `importer` に `PADDOCK_AUTO_MIGRATE=true`。api / importer Dockerfile のコメント。
- **不変**: `pool::connect` / `pool::migrate` / `pool::connect_and_migrate`（温存）。prod の起動時マイグレーション挙動（`PADDOCK_AUTO_MIGRATE=true` で従来同等）。migration ファイル自体（`deployments/db/migrations/`・リバーシブル形式）。
- **運用**: migration 追加後は共有 golden DB へ `paddock-analyze migrate` で明示適用する（起動時には適用されない）。適用は `sqlx` の advisory lock 下で並行安全。stale binary の warn が出たら最新ブランチで再ビルドする。
- 関連: #410（connect/migrate 共通化）／ADR 0069（deployments 周辺運用）。
