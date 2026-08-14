# QA — `/docs` の配信経路に機械検査を入れる（#616）

一次資料: [docs/original-docs/616-docs-serving-checks.md](../original-docs/616-docs-serving-checks.md)

## Q1: issue 本文が指定した `#[sqlx::test]` に従うべきか

- 観測/根拠: issue 本文は「app state（DB プール）付きの構築が要るので既存の `#[sqlx::test]` の
  流儀に合わせる」としている。しかし上流 `utoipa-swagger-ui` 9.0.2 を読むと、`/docs/*` のハンドラは
  `web::Path<String>` と `web::Data<Config>` しか、`/api-docs/openapi.json` のハンドラは
  `web::Data<ApiDoc>` しか抽出しない。**DB プールは `configure_routes::<R, O, S>` のジェネリクス `R` を
  具象化するためだけに要り、接続は一度も張られない**。`#[sqlx::test]` にすると、配信経路と無関係な
  理由（Postgres 未起動・migration 適用）でローカル実行が落ちる。先例として
  `tests/openapi_route_parity.rs` が `#[actix_web::test]` ＋遅延プールで同じ `configure_routes` を
  組み立てており、doc に「Postgres 不要」を設計意図として明記している。
- 回答: **確定（ユーザー承認済み）。`#[actix_web::test]` ＋遅延プールを採る。** さらにプール URL を
  **到達不能なアドレス**（`127.0.0.1:1`）にし `acquire_timeout` を 1 秒に縮める。これで「DB を触らない」が
  偶然の性質でなく**テストが強制する性質**になり、DB 依存が紛れ込めば 30 秒ハングでなく即座に落ちる。
  issue の字面から外れるが、意図（配信経路の機械検査）は満たす。
- 反映先: `src/apps/api-server/tests/docs_ui.rs` のモジュール doc / PR #618

## Q2: `/docs`（末尾スラッシュ無し）が 404 である現状をテストで固定するか

- 観測/根拠: `SwaggerUi::new("/docs/{_:.*}")` のテイルマッチは `/docs/` プレフィックスを要求するため
  `/docs` は本文空の 404（実測）。着手時点で README / 仕様書 / 起動ログ / `app.rs` の doc コメントの
  4 箇所すべてがこの 404 になる URL を案内していた。
- 回答: **確定。挙動は変えず、テストでも固定しない。** 現挙動を「期待挙動」として pin すると、将来
  リダイレクトを入れる判断をテストが阻む側に回る。**ドキュメント側だけ実態（`/docs/`）に合わせ**、
  リダイレクト要否は #619 で決める。
- 反映先: #619 / `docs_ui.rs` のモジュール doc / README / `rest-api-read.md` / `app.rs` / `bin.rs`

## Q3: この検査は「資産の取り込み失敗」を捕まえられるか

- 観測/根拠: 上流の build script は zip の展開失敗を `expect` で panic させる
  （`build.rs:53` / vendored 経路は `:163`）。つまり取り込み失敗は**コンパイル時に落ちる**。
  また `vendored` feature が脱落してもビルド時ダウンロードに戻るだけで**配信 HTML は同一**になる。
- 回答: **確定。捕まえない。** この検査が押さえるのは (a) 上流の版が上がったときの資産名・構造の
  ドリフト、(b) `SwaggerUi` の配線ミス（マウント先・spec URL・別 `ApiDoc` の混線）、
  (c) 描画元に外部オリジンが混ざる逆戻り。`vendored` の在否は
  `scripts/check-vendored-swagger.sh` の担当で、役割を分ける。
- 反映先: [ci-pipeline.md](../knowledge/ci-pipeline.md) / `docs_ui.rs` のモジュール doc

## Q4: 手動のブラウザテストはどこまで残すか

- 観測/根拠: 旧 TC-02（`/api-docs/openapi.json` が 200・JSON・`paths` 非空）は機械検査で完全に
  代替できる。一方 TC-01 の「JS 実行後に UI が組み上がるか」「コンソールにエラーが出ないか」は
  actix の in-process テストでは見られない。
- 回答: **確定。TC-02 は削除し、TC-01 は JS 描画後の画面とコンソールエラーだけに絞る。**
  「UI 一覧が spec と一致するか」も JS 描画後の話なので TC-01 の確認ポイントに含める。
- 反映先: [api-docs-swagger-ui.md](../../tests/browser-test-cases/api-docs-swagger-ui.md)

## Q5: ADR を起票するか

- 観測/根拠: 本件は ADR 0082 の follow-up で、検査を 1 本足すだけ。Q1〜Q4 の判断はいずれも
  テスト実装レベルで、アーキテクチャや運用の決定を伴わない。
- 回答: **確定。ADR は起票しない。** 判断の根拠は一次資料と `docs_ui.rs` の doc コメントに残す。
- 反映先: 一次資料の「実装方針（ADR 事項ではない）」節
