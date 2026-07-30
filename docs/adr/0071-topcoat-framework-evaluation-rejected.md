# 0071. Topcoat（tokio-rs の SSR フルスタックフレームワーク）評価 — `web/` SPA の置き換えは棄却（reject-for-now）

## ステータス

棄却（reject-for-now）。`web/` SPA の Topcoat 置き換えは採らない。コード変更なし・本 ADR は評価記録のみ。
評価基準日は **2026-07-30**。出自: セッション中の自主評価（対象 Issue なし）。
下記「再評価の条件（reject-for-now）」が満たされた時点で、本 ADR を superseded する後続 ADR を起票する
（本 ADR 自体は不変記録として残す）。

## コンテキスト

2026-07-22 に tokio-rs から [Topcoat](https://github.com/tokio-rs/topcoat) がアナウンスされた（crates.io の
初版公開は 2026-04-17 の 0.0.0）。「Rust でフルスタック reactive web app を書く」フレームワークであり、
paddock の非 Rust 部分（React SPA・Python スクリプト群・シェル）を Rust に寄せられるかを検討した。

### Topcoat の事実確認（2026-07-30 時点・出典は「関連」節に commit ピンで記載）

- crates.io 最新 **0.5.0（2026-07-27 公開）**、MIT、初版 0.0.0 は 2026-04-17、累計 DL 2,466。
  0.0.1〜0.5.0 が 7/14〜7/27 の 2 週間に 13 版＝**動きが非常に速い**。
- **完全サーバレンダリング**。async コンポーネントが DB を直接叩ける（別 API 層のボイラープレートを消す設計）。
- **WASM を使わない**。マクロで型検査済みの Rust 式を JS にクロスコンパイルし、HTMX 的な
  "reactive instructions" をメタデータとして配ることでクライアント反応性を足す。
  Leptos / Dioxus（WASM 系）とは狙う対話性の水準が違うと明言。限界時は HTMX / Alpine.js 統合にフォールバック。
- 同梱: `topcoat` CLI（dev server / `fmt` / `ui` / asset bundling）、content-hash ベースの asset pipeline、
  Tailwind ベースの shadcn/ui 風コンポーネント群、Fontsource / Iconify 統合、request-level memoization。
- **ランタイムに Node 不要**（`tailwind.md`: "It does not run Node, `PostCSS`, or a Vite-style asset pipeline"）。
  ただし**ビルド時は既定で GitHub から standalone Tailwind CLI を `OUT_DIR` にダウンロードする**。
  `BuildConfig::executable()` で preinstalled CLI を指定すれば "no download happens and no network access is needed"。
  外部 action を commit SHA でピンし cargo を `--locked` で固定する本 repo の CI 規律とは、この一点で評価が必要になる。
- ルーティングは **既定は明示パス属性**（`#[page("/users/{id}")]`）。加えて `module_router!` マクロが
  "the recommended way to define routes" として提供され、こちらは **URL をモジュール木から導出**する
  （README の例: `src/app/posts/id.rs` → `/posts/{post_id}`）。**モジュール木＝URL は推奨形であって必須ではない**。
  エントリは `topcoat::start(Router::builder().discover().build()).await`。
- README に **"Early-stage and experimental. Expect breaking changes."** と明記。
  アナウンス記事も「クライアント反応性は still in early stages」と自認。
- ルータは Topcoat 自前。optional な `tower` feature で tower service（axum router 等）を組み込める。
  README のロードマップは Toasty（tokio-rs の ORM）統合の強化ほか。MSRV は未記載。

### paddock 側の非 Rust 部分の棚卸し（tracked files）

| 領域 | 規模 | Topcoat の射程 |
|---|---|---|
| `web/` React 19 + TS + Vite SPA | `web/src` 39 files・8,904 LOC（.tsx 16 / .ts 22 / .css 1）。うち生成物 `api/schema.d.ts` 2,263 ＋ `styles.css` 990 ＋ `*.test.ts` 1,692 を除いた**手書きアプリコードは約 3,959 LOC** | ◎ 唯一の候補 |
| `scripts/predict-check/` Python | 37 files（オフライン EV レポート・backtest データ生成・各種 probe） | × Web でない |
| `tools/mdq/` Python | 17 files（BM25 ローカル索引・検索） | × 無関係 |
| `scripts/harness/` Python | 6 files（faithfulness チェック等） | × 無関係 |
| `*.sh`（リポジトリ全体） | 18 files（`deployments/` 3 / `scripts/` 直下 9 / `scripts/predict-check/` 5 / `scripts/harness/` 1） | × 無関係 |

**5 領域のうち Topcoat の射程に入るのは `web/` の 1 領域だけ**。検討対象は SPA 一点に絞られる。

## 決定

**`web/` の React SPA を Topcoat へ置き換えない。**（api-server / rest-controller も現状維持。）

## 理由

### 置き換えれば得られたはずの利点（評価済み・それでも今は取らない）

1. **型境界の消滅**。現在は `docs/api/openapi.json` → `openapi-typescript` → `web/src/api/schema.d.ts` →
   `openapi-fetch` という生成チェーンで型を渡している。Topcoat の DB 直読み構成なら DB〜画面まで単一の
   `cargo check` で通る（＝この利点は API 層ごと廃止する構成でのみ得られる。後述「却下した代替案」参照）。
2. **Node 依存木の廃棄**。react / react-router / @tanstack/react-query / openapi-fetch / vite / vitest /
   eslint / typescript ＋ `package.json` の `overrides` による脆弱性パッチ（`js-yaml`・`brace-expansion`）が消える。
3. **プロセスが 1 本に**。現在は api-server ＋ vite dev server の 2 本立ち上げ＋ `PADDOCK_API_TARGET` プロキシ設定が検証手順に入る。
4. **CI の SPA 依存の消滅**。`.github/workflows/ci.yml` の `web` ジョブ 1 本（typecheck / eslint / vitest /
   `gen:api` ドリフト検証 / `vite build` の 5 ステップ）と、`docker-build` matrix の `deployments/web.Dockerfile`
   （`node:22-slim` で `npm ci` + `npm run build`）が不要になる。

### 見送る理由

1. **0.5.0・アナウンス 8 日目・breaking changes 明言**。ライブ盤は実際に賭ける判断に使う画面であり、
   2 週間で 13 版動いている 0.x に載せるのは順序が逆。
2. **Topcoat の弱点が paddock の要求とど真ん中で衝突する**。Topcoat 自身がクライアント反応性を
   early stages と認めているが、paddock のライブ盤はまさにそこ——オッズ追従のポーリング
   （`web/src/routes/RaceBoard.tsx` の未発走ゲート付き `refetchInterval`・`web/src/routes/RaceList.tsx` の
   predict-watch スイープ追従）、`RaceBoard` / `ExecutionPanel` の対話編集（賭け金・払戻の手入力）、
   `RaceList` のソート・フィルタ（`SortTh` / `FilterChip`）。HTMX へフォールバックして書き直す価値は薄い。
   （なお `web/src/lib/useResultsRefresh.ts` はオッズではなく `POST /api/results/{date}:refresh` の
   着順取り込み／自動精算ポーリングであり、これも SSR 化すると作り直しになる。）
3. **DB 直読み構成を採ると OpenAPI 一級成果物の方針と衝突する**。`src/interface/rest-controller`（.rs 2,806 LOC）
   ＋ `src/interface/rest-controller/src/openapi.rs` ＋ `src/apps/api-server/tests/openapi.rs` /
   `openapi_route_parity.rs` の契約テスト群は、utoipa コードファーストで API 契約を担保するための投資
   （方針の出典は ADR [0022](0022-rest-api-read-server.md)）。**SSR コンポーネントが DB を直読みする構成では
   この契約自体が消える**。SPA を捨てるだけでなく actix-web + utoipa の資産を捨てる判断になる。
4. **推奨形のルーティングがレイヤ構成と当たり、HTTP スタックが 2 本になる**。paddock は
   domain / use-case / interface / apps の分離が効いている一方、Topcoat の推奨形 `module_router!` は
   「app モジュール木＝URL 木」を要求する（明示パス属性で回避はできるが、その場合は推奨形から外れる）。
   加えて Topcoat は自前ルータを持つため、actix-web の api-server と共存させると **HTTP スタックを 2 本抱える**。
5. **Tailwind 前提の同梱 UI が旨味にならない**。paddock の web は `web/src/styles.css` 1 枚の手書き
   ダークライブ盤で Tailwind を使っていない。同梱コンポーネント群は活かせず、新しいスタイル toolchain だけが増える。
6. **移行の実利が小さい——フロントは既に薄い**。`web/src/lib/*.ts` を確認した結果、`bets.ts` は
   API が返す `RecommendationBet`（＝Rust `build_portfolio` の出力）に UI 編集と 100 円単位ガードを
   重ねる純関数層であり、**買い方ロジックの second source にはなっていない**（詳細は下記「関連」）。
   つまり「フロントに紛れ込んだドメインロジックを Rust に回収できる」という移行動機は現時点の paddock には存在しない。

## 却下した代替案

- **中間案: Topcoat の SSR から既存 REST API を叩き、OpenAPI 契約を保ったまま SPA だけ置き換える**。
  見送り理由 3（契約の消滅）は回避でき、利点 2〜4（Node 依存木・プロセス・CI の SPA 依存）は取れる。
  しかし **最大の旨味である利点 1（型境界の消滅）が得られない**——Rust から自 API を HTTP で叩く形になり、
  型は結局 OpenAPI 経由で渡すので生成チェーンが Rust 側に移るだけ。その対価として 0.x の breaking change・
  Tailwind toolchain・HTTP スタック 2 本・クライアント反応性の弱さ（見送り理由 1・2・4・5）は全部残る。
  費用対効果が逆なので却下。
- **段階移行（Topcoat と SPA を並走させ画面単位で移す）**。2 つ目の HTTP スタック・2 系統のスタイル体系・
  2 系統のテスト基盤を維持期間中ずっと抱えることになり、「一時的な修正をしない」に反する。
  0.x の breaking change を並走期間中に被り続けるのも悪い。
- **Python 部分（`scripts/predict-check/` ・`tools/mdq/`）の Rust 化を Topcoat 起点で進める**。
  これらは Web でなく Topcoat と無関係。Rust 化の是非は独立した論点であり、本 ADR で混ぜない。

## 再評価の条件（reject-for-now）

以下の **いずれか** が満たされたら再評価する。

1. **Topcoat が 1.0 に到達**（breaking change の頻度が収まる）。
2. **クライアント反応性が SPA 相当になる**（対話編集・ポーリング更新・表のソート/フィルタを
   HTMX フォールバック無しで素直に書けるようになる）。

再評価時に PoC を挟むなら、**既存 SPA・api-server を不変のまま、read-only かつ対話性がほぼ無い
新規画面 1 枚だけ**（例: 回収率レポート）を Topcoat 単体で作る。既存画面の書き換えから始めない。
実測する項目:

- ランタイムに Node なしで動くか。
- domain / use-case レイヤをまたいで呼べるか（推奨形 `module_router!` と両立するか）。
- **ビルド時の Tailwind CLI をピン供給できるか**（`BuildConfig::executable()` で GitHub からの
  ダウンロードを止め、外部 action を SHA ピンする CI 規律と揃えられるか）。

## 影響

- **コード変更なし**。`web/`・api-server・rest-controller・`package.json`・CI いずれも不変。
- 本 ADR は決定記録のみ。CLAUDE.md・買い方ルール・本番定数への影響なし。

## 関連

- 出典（0.x で docs が動くため commit `a62195b` にピンする。crates.io / blog は可変）:
  [Announcing Topcoat（tokio.rs, 2026-07-22）](https://tokio.rs/blog/2026-07-22-announcing-topcoat)、
  [README](https://github.com/tokio-rs/topcoat/blob/a62195b6daea807cc9728ad800529b0aaa418b33/README.md)、
  [router.md](https://github.com/tokio-rs/topcoat/blob/a62195b6daea807cc9728ad800529b0aaa418b33/crates/topcoat/docs/router.md)、
  [tailwind.md](https://github.com/tokio-rs/topcoat/blob/a62195b6daea807cc9728ad800529b0aaa418b33/crates/topcoat/docs/tailwind.md)、
  [getting_started.md](https://github.com/tokio-rs/topcoat/blob/a62195b6daea807cc9728ad800529b0aaa418b33/crates/topcoat/docs/getting_started.md)、
  [crates.io: topcoat](https://crates.io/crates/topcoat)、[HN 議論](https://news.ycombinator.com/item?id=48952067)
- API 契約の方針: ADR [0022](0022-rest-api-read-server.md)（OpenAPI を一級成果物とし、utoipa コードファースト＋
  `docs/api/openapi.json` のスナップショット検証で担保する決定）。実装は `src/interface/rest-controller/`。
- 買い方ロジックの二重実装: ADR [0064](0064-live-ev-buy-view.md) は**当時の正本を `live_ev.py` に一本化**し、
  「Rust domain や TS に再実装すると second source が生まれる」と警告した。その後 #346 で writer が
  Rust `predict-watch` / `build_portfolio` に一本化され、**正本が Rust 側へ移って `live_ev.py` が
  second source として残った**（現行の CLAUDE.md「予算・配分」の記述がこれ）。本 ADR は、`web/src/lib/bets.ts` が
  この二重実装に**当たらない**（配分・混戦判定・組み合わせ生成を持たず、`RecommendationBet` への
  UI 編集と 100 円単位ガードのみ）ことを確認した記録も兼ねる。
- 同型のステータス運用の先例: ADR [0067](0067-late-money-odds-drift-signal-rejected.md)（棄却（reject-for-now）＋再検証の条件）。
