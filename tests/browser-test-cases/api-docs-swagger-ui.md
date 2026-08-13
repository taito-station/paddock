# ブラウザテストケース: `/docs` の Swagger UI（vendored 化・#606 論点 B）

対象: `src/apps/api-server/src/app.rs` の `SwaggerUi::new("/docs/{_:.*}")`。
ADR [0082](../../docs/original-docs/0082-swagger-ui-vendored.md) / knowledge
[ci-pipeline.md](../../docs/knowledge/ci-pipeline.md)。

**この 2 ケースの目的は「ビルド時ダウンロードをやめても `/docs` が従来どおり動く」ことの確認**。
埋め込み版は従来のダウンロード版と**同一**（`utoipa-swagger-ui-vendored` 0.1.2 が同梱する
`res/v5.17.14.zip` ＝既定のダウンロード先と同じ v5.17.14 タグ）なので資産が変わる理屈は無いが、
**バイナリが起動して 200 を返すだけでは「資産が壊れていない」の証明にならない**ので、描画と
エンドポイント一覧を目視する（埋め込みの取り込みに失敗すれば空の UI やスクリプトエラーになる）。

検証環境は Playwright MCP 不在のため **headless Chrome + puppeteer-core** で代替する
（`reference_browser_test_fallback`）。api-server は golden DB を read-only 参照でソースから起動し、
`PADDOCK_SERVER_ADDR` で :8081 に寄せる（`reference_web_verify_stack`）。DB は一切書き換えない。

---

### TC-01: `/docs/` が Swagger UI として描画され、API のエンドポイント一覧が出る
| 項目 | 内容 |
|------|------|
| 前提 | `cargo run -p api-server`（`PADDOCK_SERVER_ADDR=127.0.0.1:8081`）が起動済み。`vendored` feature 有効でビルドされている |
| 画面 | `http://127.0.0.1:8081/docs/` |
| 操作 | ページを開き、描画完了を待つ。スクリーンショットを撮る。タグ（`races` 等）を 1 つ展開する |
| 期待結果 | Swagger UI の外枠（トップバー・タイトル）が描画される。**paddock の API のパスが 1 件以上一覧される**（空の "No operations defined in spec!" ではない）。タグを展開するとオペレーションの詳細が開く |
| 確認ポイント | ブラウザコンソールに 4xx/5xx やスクリプトエラーが出ていないこと（vendored の資産欠落はここに出る。`favicon.ico` の 404 は vendored とは無関係の既存事象） |

### TC-02: `/api-docs/openapi.json` が 200 で JSON を返し、UI の描画元と一致する
| 項目 | 内容 |
|------|------|
| 前提 | TC-01 と同じ |
| 画面 | `http://127.0.0.1:8081/api-docs/openapi.json` |
| 操作 | 直接開く（または TC-01 のページ読み込み時のネットワークログから当該リクエストを拾う） |
| 期待結果 | HTTP 200・`Content-Type` が JSON。`openapi` / `paths` キーを持ち、`paths` が空でない |
| 確認ポイント | TC-01 で一覧されたパスと `paths` のキーが一致すること（UI が別の spec を読んでいないことの確認）。`cargo test -p api-server` の openapi スナップショットが検証しているのは仕様生成側なので、**配信経路の確認はここで行う** |
