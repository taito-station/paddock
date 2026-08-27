# 616 — `/docs`（Swagger UI）の配信経路に機械検査が無い（生資料）

ADR 0082（PR #614）で Swagger UI をビルド時ダウンロードから `utoipa-swagger-ui-vendored` の埋め込み
資産へ載せ替えた際のセルフレビューで、**`/docs` の配信経路に機械検査が無い**ことが分かった。
issue 本文は [gh issue view 616](https://github.com/taito-station/paddock/issues/616)。

質問票: [QA-docs-serving-checks-616.md](../qa/QA-docs-serving-checks-616.md)
蒸留先: [ci-pipeline.md](../knowledge/ci-pipeline.md) / `src/apps/api-server/tests/docs_ui.rs` の
モジュール doc / [api-docs-swagger-ui.md](../../tests/browser-test-cases/api-docs-swagger-ui.md)。

## 検査の空白（着手時点）

| 既存の検査 | 見ているもの | `/docs` の配信 |
|---|---|---|
| `tests/openapi.rs` | `docs/api/openapi.json` のスナップショット | 対象外（生成側） |
| `tests/openapi_route_parity.rs` | `ApiDoc.paths` ↔ `REGISTERED_ROUTES` | 対象外（`/api` のみ） |
| `scripts/check-vendored-swagger.sh` | `vendored` feature の在否（`Cargo.lock` ＋宣言） | 対象外（宣言のみ） |
| `tests/browser-test-cases/api-docs-swagger-ui.md` | 描画とエンドポイント一覧 | **手動のみ** |

## 上流実装の実測（`utoipa-swagger-ui` 9.0.2）

テスト設計の前提として確認した事実。

- `/docs/*` のハンドラ（`serve_swagger_ui`）は `web::Path<String>` と `web::Data<Config>` しか、
  `/api-docs/openapi.json` のハンドラ（`get_api_doc`）は `web::Data<ApiDoc>` しか抽出しない。
  **DB プールは `configure_routes::<R, O, S>` のジェネリクス具象化にしか要らず、接続は張られない**。
- `serve()` はキャプチャが `""` または `"/"` のとき `index.html` にフォールバックする
  （＝`/docs/` は 200）。資産が引けなければ `HttpResponse::NotFound()`。
- `swagger-initializer.js` のときだけ `{{config}}` を `Config` の JSON へ置換する。
  **上流既定の描画元は petstore** なので、置換が壊れると UI は他人の spec を読む。
- zip の展開失敗は build script の `expect`（`build.rs:53` / vendored 経路は `:163`）で **panic ＝
  コンパイル失敗**になる。**「資産の取り込み失敗」はランタイムの 404 として現れない**。

## 配信される資産（v5.17.14・実測）

`index.html`（734 B）が読み込むのは 5 本。1 本でも欠けると UI が起動しない。

| 資産 | 実測サイズ | 欠けたときの症状 |
|---|---|---|
| `swagger-ui.css` | 152,071 B | 無スタイル描画 |
| `index.css` | 202 B | レイアウト崩れ |
| `swagger-ui-bundle.js` | 1,452,753 B | `SwaggerUIBundle is not defined` |
| `swagger-ui-standalone-preset.js` | 230,293 B | `SwaggerUIStandalonePreset is not defined`（画面が真っ白） |
| `swagger-initializer.js` | 423 B | UI が起動しない |

## `/docs`（末尾スラッシュ無し）は 404

`SwaggerUi::new("/docs/{_:.*}")` のテイルマッチは `/docs/` プレフィックスを要求するため、`/docs` は
どのリソースにもマッチせず**本文空の 404** になる（`bin.rs` に `NormalizePath` も無い）。実測で確認。

着手時点で `README.md` / `docs/specifications/rest-api-read.md` / 起動ログ / `app.rs` の doc コメントの
**4 箇所すべてが `/docs`（404 になる URL）で案内していた**。扱いは #619。

## 退行検知力の実証（mutation）

追加した検査が「常に通る壊れ方」をしていないことを、本番側を意図的に壊して確認した。

| 改変 | 落ちたテスト |
|---|---|
| `SwaggerUi::url` を `/api-docs/MUTATED.json` へ | 描画元の検査・spec 配信の検査（2 本） |
| マウントを `/mutated/{_:.*}` へ | `/docs` 系 3 本（`/api-docs` 系は影響なし） |

いずれも改変を戻して green に復帰することまで確認。

## ブラウザ実測（headless Chrome・2026-08-14）

- `/docs/` が描画され **22 パス / 23 オペレーション**が一覧された
- 描画されたパス集合が `/api-docs/openapi.json` の `paths` と**両方向で完全一致**（差分 0）
- ページ由来のコンソール出力なし。api-server 側のログにも 4xx/5xx なし

## 実装方針（ADR 事項ではない）

いずれもテスト実装レベルの選択で、アーキテクチャや運用の決定を伴わないため **ADR は起票しない**
（ADR 0082 の follow-up）。決定記録は ADR に限る規約と混同しないよう、ここは「方針」として残す。

- テストは `#[sqlx::test]` ではなく `#[actix_web::test]` ＋ **到達不能アドレスへの遅延プール**で書く
  （issue 本文の指定から逸脱）。理由は上記のとおり配信経路が DB を触らないため。到達不能にするのは
  「DB を触らない」をテストが強制する性質にするため
- `/docs` の 404 は**テストで固定しない**。pin すると #619 でリダイレクトを入れる判断を阻む
