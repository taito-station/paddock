# 0071. Topcoat（tokio-rs の SSR フルスタックフレームワーク）評価 — `web/` SPA の置き換えは棄却（reject-for-now）

## ステータス

棄却（reject-for-now）。`web/` SPA の Topcoat 置き換えは採らない。コード変更なし・本 ADR は評価記録のみ。
評価基準日は **2026-07-30**。下記「再評価の条件（reject-for-now）」が満たされたら再評価する。
**再評価の結果として決定が変わる場合に限り**、本 ADR を superseded する後続 ADR を起票する
（再評価しても棄却なら本 ADR を維持）。本 ADR 自体は不変記録として残す。

## コンテキスト

2026-07-22 に tokio-rs から [Topcoat](https://github.com/tokio-rs/topcoat) がアナウンスされた。
「Rust でフルスタック reactive web app を書く」フレームワークであり、paddock の非 Rust 部分
（React SPA・Python スクリプト群・シェル）を Rust に寄せられるかを検討した。

### Topcoat の事実確認（2026-07-30 時点・出典は「関連」節に commit ピンで記載）

- crates.io 最新 **0.5.0（2026-07-27 公開）**、MIT、初版 0.0.0 は 2026-04-17、累計 DL 2,466。
  0.0.1〜0.5.0 が 2026-07-14〜2026-07-27 の 2 週間に 13 版＝**動きが非常に速い**。
- **MSRV は crates.io メタデータで 1.95**（README には記載がない。edition 2024）。
  本 repo の `rust-toolchain.toml` は 1.97.1 なので**充足済み＝評価上の懸念ではない**。
- **完全サーバレンダリング**。async コンポーネントが DB を直接叩ける（別 API 層のボイラープレートを消す設計）。
- **WASM を使わない**。マクロで型検査済みの Rust 式を JS にクロスコンパイルし、HTMX 的な
  "reactive instructions" をメタデータとして配ることでクライアント反応性を足す。
  Leptos / Dioxus（WASM 系）とは狙う対話性の水準が違うと明言。限界時は HTMX / Alpine.js 統合にフォールバック。
- 同梱: `topcoat` CLI（dev server / `fmt` / `ui` / asset bundling）、content-hash ベースの asset pipeline、
  Tailwind ベースの shadcn/ui 風コンポーネント群、Fontsource / Iconify 統合、request-level memoization。
- **ランタイムに Node 不要**（完全サーバレンダリング＋WASM 非使用という上記 2 点の帰結）。
- **Tailwind 統合は default 外の opt-in feature**。crates.io 0.5.0 の `default` は
  `asset` / `compression` / `cookie` / `font` / `icon` / `router` / `runtime` / `serve` / `session` / `view` / `discover` で、
  `tailwind` は `dep:topcoat-tailwind` の optional（`tailwind.md`:「Enable the `tailwind` feature for both your
  runtime dependency and your build dependency」＋ `build.rs` の追加が必要）。
  **この feature を有効にした場合に限り**、ビルド時に既定で GitHub から standalone Tailwind CLI を
  `OUT_DIR` にダウンロードする（Tailwind 統合は "a thin Rust wrapper around the standalone Tailwind CSS CLI" で
  "It does not run Node, `PostCSS`, or a Vite-style asset pipeline"）。`BuildConfig::executable()` で
  preinstalled CLI を指定すれば "no download happens and no network access is needed"。
  外部 action を commit SHA でピンし cargo を `--locked` で固定する本 repo の CI 規律との擦り合わせが必要なのは、
  **`tailwind` feature を使う場合だけ**である。
- ルーティングは **既定は明示パス属性**（`#[page("/users/{id}")]`）。加えて `module_router!` マクロが
  "the recommended way to define routes" として提供され、こちらは **URL をモジュール木から導出**する
  （README の例: `src/app/posts/id.rs` → `/posts/{post_id}`）。**モジュール木＝URL は推奨形であって必須ではない**。
  エントリは `topcoat::start(Router::builder().discover().build()).await`。
- ルータは Topcoat 自前。optional な `tower` feature で tower service（axum router 等）を組み込める。
- README に **"Early-stage and experimental. Expect breaking changes."** と明記。
  アナウンス記事も「クライアント反応性は still in early stages」と自認。
- README のロードマップには **`OpenAPI` endpoints**、**(More) reactivity（`topcoat-runtime`）**、**Islands**、
  Streaming SSR / Suspense、Client-side navigation + prefetching、Toasty（tokio-rs の ORM）統合の強化、
  Static export、Authentication 等が並ぶ。**下記の見送り理由 2 と 3 は、このうち
  `topcoat-runtime` / Islands と `OpenAPI` endpoints が実装されれば弱まる**——再評価の観測点はここに置く。

### paddock 側の非 Rust 部分の棚卸し（tracked files・実測基準 `main` = `409e4a4`）

| 領域 | 規模 | Topcoat の射程 |
|---|---|---|
| `web/` React 19 + TS + Vite SPA | `web/src` 39 files・8,904 LOC（.tsx 16 / .ts 22 / .css 1）。内訳は生成物 `web/src/api/schema.d.ts` 2,263 ／ 手書き CSS `web/src/styles.css` 990 ／ テスト `*.test.ts` 1,692 ／ **手書きアプリコード 3,959 LOC**。ほかに `web/` 直下の設定 7 files（`package.json` / `vite.config.ts` / `eslint.config.js` / `tsconfig.json` / `index.html` / `package-lock.json` / `.gitignore`） | ◎ 唯一の候補 |
| `scripts/predict-check/` | `.py` 37 files（オフライン EV レポート・backtest データ生成・各種 probe） | × Web でない |
| `tools/mdq/` | `.py` 17 files（BM25 ローカル索引・検索） | × 無関係 |
| `scripts/harness/` | `.py` 6 files（faithfulness チェック等） | × 無関係 |
| シェル（横断カテゴリ・上記ディレクトリと一部重なる） | `*.sh` 18 files（`deployments/` 3 / `scripts/` 直下 9 / `scripts/predict-check/` 5 / `scripts/harness/` 1）＋ 拡張子なしの bash 2 files（`scripts/mdq` / `scripts/git-hooks/pre-push`） | × 無関係 |

Python 行の件数は `.py` のみの数（tracked 総数は順に 43 / 20 / 10）。最終行はディレクトリ横断のカテゴリで、
上のディレクトリ行と一部重なる。**Topcoat の射程に入るのは `web/` だけ**で、検討対象は SPA 一点に絞られる。

本 ADR に載せた LOC / ファイル数は**すべて `main` = `409e4a4` 時点のスナップショット**である
（ADR は不変記録なので後続の変更で追随させない。再測するときはこの基準 commit と比較する）。

## 決定

**`web/` の React SPA を Topcoat へ置き換えない。**（api-server / rest-controller も現状維持。）

## 理由

### 置き換えれば得られたはずの利点（評価済み・それでも今は取らない）

1. **型境界の消滅**。現在は `docs/api/openapi.json` → `openapi-typescript` → `web/src/api/schema.d.ts` →
   `openapi-fetch` という生成チェーンで型を渡している。Topcoat の DB 直読み構成なら DB〜画面まで単一の
   `cargo check` で通る（＝この利点は API 層ごと廃止する構成でのみ得られる。後述「却下した代替案」参照）。
2. **Node 依存木の廃棄**。react / react-router / @tanstack/react-query / openapi-fetch / vite / vitest /
   eslint / typescript ＋ `package.json` の `overrides` による脆弱性パッチ（`js-yaml`・`brace-expansion`）が消える。
3. **プロセスが 1 本に**。現在は api-server ＋ vite dev server の 2 本立ち上げが検証手順に入る
   （`web/vite.config.ts` の proxy 先は既定 `http://localhost:8080`。`PADDOCK_API_TARGET` はポート競合時の任意上書き）。
4. **CI の SPA 依存の消滅**。`.github/workflows/ci.yml` の `web` ジョブ 1 本（全 8 ステップ。うち検査は
   typecheck / eslint / vitest / `gen:api` ドリフト検証 / `vite build` の 5 で、残りは checkout / setup-node / `npm ci`）と、
   `docker-build` matrix の `deployments/web.Dockerfile`（`node:22-slim` で `npm ci` + `npm run build`）が不要になる。

### 見送る理由

1. **0.5.0・アナウンスから 8 日・breaking changes 明言**。ライブ盤は実際に賭ける判断に使う画面であり、
   2 週間で 13 版動いている 0.x に載せるのは順序が逆。
2. **Topcoat の弱点が paddock の要求とど真ん中で衝突する**。Topcoat 自身がクライアント反応性を
   early stages と認めているが、paddock のライブ盤はまさにそこ——オッズ追従のポーリング
   （`web/src/routes/RaceBoard.tsx` の未発走ゲート付き `refetchInterval`・`web/src/routes/RaceList.tsx` の
   predict-watch スイープ追従）、`RaceBoard` / `ExecutionPanel` の対話編集（賭け金・払戻の手入力）、
   `RaceList` のソート・フィルタ（`SortTh` / `FilterChip`）。HTMX へフォールバックして書き直す価値は薄い。
   （なお `web/src/lib/useResultsRefresh.ts` はオッズではなく `POST /api/results/{date}:refresh` の
   着順取り込み／自動精算ポーリングであり、これも SSR 化すると作り直しになる。）
3. **DB 直読み構成を採ると OpenAPI 一級成果物の方針と衝突する**（＝理由 4 後段とは排他の分岐で、
   こちらは api-server を廃止する側）。`src/interface/rest-controller`
   （.rs 2,730 LOC。この LOC には同 crate の `src/openapi.rs` も含む）と、`src/apps/api-server/tests/openapi.rs` /
   `openapi_route_parity.rs` の契約テスト 2 本は、utoipa コードファーストで API 契約を担保するための投資
   （方針の出典は ADR [0022](0022-rest-api-read-server.md)）。**SSR コンポーネントが DB を直読みする構成では
   この契約自体が消える**。SPA を捨てるだけでなく actix-web + utoipa の資産を捨てる判断になる。
   （Topcoat のロードマップには `OpenAPI` endpoints があるため、実装されればこの理由は弱まる。現状は未実装。）
4. **推奨形のルーティングがレイヤ構成と当たり、HTTP スタックが 2 本になる**。paddock は
   `src/` 直下を domain / use-case / interface / infrastructure / apps の 5 層に分けており
   （ADR 0064 の「rest-controller・use-case・rdb-gateway・api-server の 4 層」は read API 1 本が貫く
   crate の列挙で、この 5 層とは別の切り口）、
   Topcoat の推奨形 `module_router!` は「app モジュール木＝URL 木」を要求する
   （明示パス属性で回避はできるが、その場合は推奨形から外れる）。
   加えて **api-server を残す分岐（＝理由 3 とは排他）では HTTP スタックを 2 本抱える**。Topcoat は自前ルータを持ち、
   optional な `tower` feature で組み込めるのは tower service（axum router 等）であって
   **actix-web はこれに該当しないため、feature では 1 本に畳めない**。
5. **Tailwind 前提の同梱 UI が旨味にならない**。paddock の web は `web/src/styles.css` 1 枚の手書き
   ダークライブ盤で Tailwind を使っていない。同梱の shadcn/ui 風コンポーネント群は活かせないので、
   **"batteries-included" の売りのうちこの分は利点 0**。ただし `tailwind` feature は default 外なので、
   切っておけばコスト増にはならない——これは減点ではなく「移行の動機が 1 つ減る」という意味に留める。
6. **移行の実利が小さい——フロントは薄い**。`web/src/lib/bets.ts` は API が返す `RecommendationBet`
   （＝Rust `build_portfolio` の出力）に UI 編集と 100 円単位ガードを重ねる純関数層であり、
   **買い方ロジックの second source にはなっていない**（詳細は下記「関連」）。
   ただし**ルール由来の定数・判定が TS 側に少量ある**のは事実で、ここは正確に記録しておく:
   `web/src/lib/live.ts` の `DANZEN_WIN_ODDS_MAX = 1.9`（CLAUDE.md「断然人気は EV がマイナスになりがち」由来）と
   それを使う `skipReason()`、`SOON_MINUTES = 20` / `STALE_MINUTES = 10`（predict-watch の窓 40 分・間隔 5 分由来）、
   `web/src/lib/board.ts` の `DEFAULT_RACE_BUDGET = 5000`、`web/src/lib/constants.ts` の
   `DEFAULT_SESSION_BUDGET` / ポーリング間隔。**これらは表示用の閾値・既定値であって配分・混戦判定・
   組み合わせ生成のロジックではない**ため、Rust 回収の価値は「あるが小さい」。フレームワーク移行を
   正当化する規模の移行動機ではない、が正確な言い方。

## 却下した代替案

- **中間案: Topcoat の SSR から既存 REST API を叩き、OpenAPI 契約を保ったまま SPA だけ置き換える**。
  見送り理由 3（契約の消滅）は回避でき、**利点 2・4（Node 依存木・CI の SPA 依存）は取れる**。
  一方で **利点 3（プロセス 1 本化）は原理的に得られない**——Topcoat サーバと actix-web の api-server で
  dev / prod とも 2 プロセスが残る。そして **最大の旨味である利点 1（型境界の消滅）は部分的にしか得られない**：
  両端が Rust なので `src/interface/rest-controller/src/schema/*`（utoipa の Rust 型）を crate 依存で
  共有すれば **codegen（`openapi-typescript` → `schema.d.ts`）自体は消せる**。ただし
  **in-process 呼び出しでない限り HTTP シリアライズ境界と 2 プロセスの運用は残る**ため、
  利点 1 の本体である「DB〜画面まで単一の型検査」には届かない。
  対価として 0.x の breaking change・HTTP スタック 2 本・クライアント反応性の弱さ
  （見送り理由 1・2・4）は全部残る。費用対効果が逆なので却下。
- **段階移行（Topcoat と SPA を並走させ画面単位で移す）**。2 つ目の HTTP スタック・2 系統のスタイル体系・
  2 系統のテスト基盤を維持期間中ずっと抱えることになり、「一時的な修正をしない」に反する。
  0.x の breaking change を並走期間中に被り続けるのも悪い。
- **Python 部分（`scripts/predict-check/`・`tools/mdq/`）の Rust 化を Topcoat 起点で進める**。
  これらは Web でなく Topcoat と無関係。Rust 化の是非は独立した論点であり、本 ADR で混ぜない。

## 再評価の条件（reject-for-now）

**再評価の起点になるのは 1 または 2 のいずれか**（3 は単独では起点にせず、1 / 2 と併せて見る補助的観測点）。
それまでは棄却を維持する。

1. **Topcoat が 1.0 に到達**（breaking change の頻度が収まる）。
2. **クライアント反応性が SPA 相当になる**。観測点はロードマップの
   **(More) reactivity（`topcoat-runtime`）と Islands**、および Client-side navigation + prefetching。
   対話編集・ポーリング更新・表のソート/フィルタを HTMX フォールバック無しで素直に書けるかで判定する。
3. （補助的な観測点）**ロードマップの `OpenAPI` endpoints が実装される**。見送り理由 3 が弱まるため、
   1 または 2 と併せて再評価の後押しになる。

再評価時に PoC を挟むなら、**既存 SPA・api-server を不変のまま、read-only かつ対話性がほぼ無い
新規画面 1 枚だけ**（例: 回収率レポート）を Topcoat 単体で作る。既存画面の書き換えから始めない。
実測する項目:

- ランタイムに Node なしで動くか。
- domain / use-case レイヤをまたいで呼べるか（推奨形 `module_router!` と両立するか）。
- **`tailwind` feature を切ったまま成立するか**（paddock は手書き CSS なのでこれが既定路線）。
  使う判断になった場合のみ、**ビルド時の Tailwind CLI をピン供給できるか**を確認する
  （`BuildConfig::executable()` で GitHub からのダウンロードを止め、外部 action を SHA ピンする CI 規律と揃えられるか）。

## 影響

- **コード変更なし**。`web/`・api-server・rest-controller・`package.json`・CI いずれも不変。
- 本 ADR は決定記録のみ。CLAUDE.md・買い方ルール・本番定数への影響なし。

## 関連

- 出自: セッション中の自主評価（対象 Issue なし）。
- 出典（0.x で docs が動くため GitHub 上のものは commit `a62195b` にピンする）:
  [Announcing Topcoat（tokio.rs, 2026-07-22）](https://tokio.rs/blog/2026-07-22-announcing-topcoat)、
  [README](https://github.com/tokio-rs/topcoat/blob/a62195b6daea807cc9728ad800529b0aaa418b33/README.md)、
  [router.md](https://github.com/tokio-rs/topcoat/blob/a62195b6daea807cc9728ad800529b0aaa418b33/crates/topcoat/docs/router.md)、
  [tailwind.md](https://github.com/tokio-rs/topcoat/blob/a62195b6daea807cc9728ad800529b0aaa418b33/crates/topcoat/docs/tailwind.md)、
  [getting_started.md](https://github.com/tokio-rs/topcoat/blob/a62195b6daea807cc9728ad800529b0aaa418b33/crates/topcoat/docs/getting_started.md)、
  [crates.io: topcoat](https://crates.io/crates/topcoat)、[HN 議論](https://news.ycombinator.com/item?id=48952067)。
  **crates.io 由来の数値（0.5.0 / 2026-07-27・累計 DL 2,466・13 版）と HN スレッドはピンできないため、
  評価基準日 2026-07-30 のスナップショットであり後から厳密には再現できない**（版一覧は crates.io API で再取得可）。
- 置き換え対象の仕様: [docs/specifications/web-spa.md](../specifications/web-spa.md)（`status: Confirmed`）、
  ADR [0069](0069-drop-icloud-writes-browser-only-viewing.md)（iCloud 書き出しを全廃し閲覧を REST API + SPA に一本化）。
  **なお web-spa.md の鮮度方針は「既定は自動ポーリングしない／恒常的な全画面ポーリングはやらない。例外は
  `results:refresh`（#381・ADR [0068](0068-race-result-ingestion-ui-reflection.md)）だけ」となっており、実装済みの RaceBoard（#475）・RaceList（#372）の
  オッズ追従ポーリングを反映していない（spec が stale ＝ CLAUDE.md の `Conflict` 相当）**。本 ADR の
  見送り理由 2 は実装側を事実として採っている。この spec 更新は本 ADR のスコープ外なので
  **追跡 Issue [#567](https://github.com/taito-station/paddock/issues/567) で解消する**。
- API 契約の方針: ADR [0022](0022-rest-api-read-server.md)（OpenAPI を一級成果物とし、utoipa コードファースト＋
  `docs/api/openapi.json` のスナップショット検証で担保する決定）。実装は `src/interface/rest-controller/`。
- 買い方ロジックの二重実装: ADR [0064](0064-live-ev-buy-view.md) は**当時の正本を `live_ev.py` に一本化**し、
  「Rust domain や TS に再実装すると second source が生まれる」と警告した。その後 #346 で writer が
  Rust `predict-watch` / `build_portfolio` に一本化され、**正本が Rust 側へ移って `live_ev.py` が
  second source として残った**（現行の CLAUDE.md「予算・配分」の記述がこれ）。本 ADR は、`web/src/lib/bets.ts` が
  この二重実装に**当たらない**（配分・混戦判定・組み合わせ生成を持たず、`RecommendationBet` への
  UI 編集と 100 円単位ガードのみ）ことを確認した記録も兼ねる。
- 同型のステータス運用の先例: ADR [0067](0067-late-money-odds-drift-signal-rejected.md)（棄却（reject-for-now）＋再検証の条件）。
