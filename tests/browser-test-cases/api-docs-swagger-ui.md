# ブラウザテストケース: `/docs` の Swagger UI（vendored 化・#606 論点 B）

対象: `src/apps/api-server/src/app.rs` の `SwaggerUi::new("/docs/{_:.*}")`。
ADR [0082](../../docs/docs-original/0082-swagger-ui-vendored.md) / knowledge
[ci-pipeline.md](../../docs/knowledge/ci-pipeline.md)。

**HTTP 層は機械検査が見る**（`src/apps/api-server/tests/docs_ui.rs`・#616）。`index.html` /
`swagger-initializer.js` / UI 本体 JS / `openapi.json` の配信と本文、および未知の資産が 404 になること
までは CI で毎回固定されるので、**手動で見直す必要はない**。

**このケースに残る役割は、機械検査では見られない「JS を実行した後」だけ**——資産が配信されていても
UI が組み上がるとは限らないため、描画結果とブラウザコンソールを目視する。

検証環境は Playwright MCP 不在のため **headless Chrome + puppeteer-core** で代替する
（`reference_browser_test_fallback`）。api-server は golden DB を read-only 参照でソースから起動し、
`PADDOCK_SERVER_ADDR` で :8081 に寄せる（`reference_web_verify_stack`）。DB は一切書き換えない。

---

### TC-01: `/docs/` が Swagger UI として描画され、API のエンドポイント一覧が出る
| 項目 | 内容 |
|------|------|
| 前提 | `cargo run -p api-server`（`PADDOCK_SERVER_ADDR=127.0.0.1:8081`）が起動済み。`vendored` feature 有効でビルドされている |
| 画面 | `http://127.0.0.1:8081/docs/` |
| 操作 | ① ページを開き描画完了を待つ ② スクリーンショットを撮る ③ タグ（`races` 等）を 1 つ展開する ④ `http://127.0.0.1:8081/api-docs/openapi.json` を別タブで開く（または DevTools のネットワークログから当該レスポンスを拾う）——**④ は下の突合に使う比較材料の取得であって、spec 単体の検査ではない**（200・JSON・`paths` 非空は機械検査済み） |
| 期待結果 | Swagger UI の外枠（トップバー・タイトル）が描画される。**paddock の API のパスが 1 件以上一覧される**（空の "No operations defined in spec!" ではない）。タグを展開するとオペレーションの詳細が開く |
| 確認ポイント | ブラウザコンソールに 4xx/5xx やスクリプトエラーが出ていないこと（`favicon.ico` の 404 は vendored とは無関係の既存事象）。一覧されたパスが `/api-docs/openapi.json` の `paths` と一致すること（UI が別の spec を読んでいないことの確認・旧 TC-02 の残り） |

> **`/api-docs/openapi.json` の配信確認（旧 TC-02）は機械検査へ移した**（#616）。200・JSON・`paths`
> 非空は `docs_ui.rs` が毎回見るので、**単体検査としては**開き直す必要はない（比較材料として開くのは操作 ④ のとおり）。UI に一覧されたパスが spec と
> 一致するか（＝UI が別の spec を読んでいないか）だけは JS 描画後の話なので TC-01 の観点に含める。
