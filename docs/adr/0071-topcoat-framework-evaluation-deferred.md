# 0071. Topcoat（tokio-rs の SSR フルスタックフレームワーク）評価 — `web/` SPA の置き換えは現時点で見送り

## ステータス

見送り（defer）。コード変更なし・本 ADR は評価記録のみ。評価基準日は **2026-07-30**。
再評価の条件は下記「再評価の条件」。満たされた時点で本 ADR を上書きする後続 ADR を起票する。

## コンテキスト

2026-07-22 に tokio-rs から [Topcoat](https://github.com/tokio-rs/topcoat) が公開された。
「Rust でフルスタック reactive web app を書く」フレームワークであり、paddock の非 Rust 部分
（React SPA・Python スクリプト群・シェル）を Rust に寄せられるかを検討した。

### Topcoat の事実確認（2026-07-30 時点）

- crates.io 最新 **0.5.0（2026-07-27 公開）**、MIT、初版 0.0.0 は 2026-04-17、累計 DL 2,466。
  0.0.1〜0.5.0 が 7/14〜7/27 の 2 週間に 13 版＝**動きが非常に速い**。
- **完全サーバレンダリング**。async コンポーネントが DB を直接叩ける（別 API 層のボイラープレートを消す設計）。
- **WASM を使わない**。マクロで型検査済みの Rust 式を JS にクロスコンパイルし、HTMX 的な
  "reactive instructions" をメタデータとして配ることでクライアント反応性を足す。
  Leptos / Dioxus（WASM 系）とは狙う対話性の水準が違うと明言。限界時は HTMX / Alpine.js 統合にフォールバック。
- 同梱: `topcoat` CLI（dev server / `fmt` / `ui` / asset bundling）、content-hash ベースの asset pipeline、
  Tailwind ベースの shadcn/ui 風コンポーネント群、Fontsource / Iconify 統合、request-level memoization。**Node 不要**。
- ルーティングは **モジュール構造＝URL 構造**（`src/app/posts/id.rs` → `/posts/{post_id}`）。
  エントリは `topcoat::start(Router::builder().discover().build()).await`。
- README に **"Early-stage and experimental. Expect breaking changes."** と明記。
  アナウンス記事も「クライアント反応性は still in early stages」と自認。
- Toasty（ORM・2026-04 リリース）と Axum への統合が今後の予定。MSRV は未記載。

### paddock 側の非 Rust 部分の棚卸し（tracked files）

| 領域 | 規模 | Topcoat の射程 |
|---|---|---|
| `web/` React 19 + TS + Vite SPA | `web/src` 8,904 LOC（.tsx 16 / .ts 23、うち vitest が多数） | ◎ 唯一の候補 |
| `scripts/predict-check/` Python | 37 files（オフライン EV レポート・backtest データ生成・各種 probe） | × Web でない |
| `tools/mdq/` Python | 17 files（BM25 ローカル索引・検索） | × 無関係 |
| `scripts/harness/` Python ＋ `*.sh` | 6 ＋ 18 files（運用・DB・launchd・バックアップ） | × 無関係 |

**非 Rust 部分の 3/4 は Topcoat の射程外**。検討対象は SPA 一点に絞られる。

## 決定

**`web/` の React SPA を Topcoat へ置き換えない。**（api-server / rest-controller も現状維持。）

## 理由

### 置き換えれば得られたはずの利点（評価済み・それでも今は取らない）

1. **型境界の消滅**。現在は `docs/api/openapi.json` → `openapi-typescript` → `web/src/api/schema.d.ts` →
   `openapi-fetch` という生成チェーンで型を渡している。Topcoat なら DB〜画面まで単一の `cargo check` で通る。
2. **Node 依存木の廃棄**。react / react-router / @tanstack/react-query / openapi-fetch / vite / vitest /
   eslint / typescript ＋ `package.json` の `overrides` による脆弱性パッチ（`js-yaml`・`brace-expansion`）が消える。
3. **プロセスが 1 本に**。現在は api-server ＋ vite dev server の 2 本立ち上げ＋ `PADDOCK_API_TARGET` プロキシ設定が検証手順に入る。
4. **CI レーンの削減**。`tsc` / `eslint` / `vitest` / `gen:api` の 4 レーンが cargo だけになる。

### 見送る理由

1. **0.5.0・アナウンス 8 日目・breaking changes 明言**。ライブ盤は実際に賭ける判断に使う画面であり、
   2 週間で 13 版動いている 0.x に載せるのは順序が逆。
2. **Topcoat の弱点が paddock の要求とど真ん中で衝突する**。Topcoat 自身がクライアント反応性を
   early stages と認めているが、paddock のライブ盤はまさにそこ——オッズポーリング（`useResultsRefresh`）、
   `RaceBoard` / `ExecutionPanel` の対話編集（賭け金・払戻の手入力）、`RaceList` のソート・フィルタ
   （`SortTh` / `FilterChip`）。HTMX へフォールバックして書き直す価値は薄い。
3. **OpenAPI 一級成果物の方針と衝突する**。`rest-controller` 2,820 LOC ＋ `rest-controller/src/openapi.rs` ＋
   `api-server/tests/openapi.rs` / `openapi_route_parity.rs` の契約テスト群は、utoipa コードファーストで API 契約を
   担保するための投資。SSR コンポーネントが DB を直読みする形にすると **この契約自体が消える**。
   SPA を捨てるだけでなく actix-web + utoipa の資産を捨てる判断になる。
4. **モジュール＝ルートの規約がレイヤ構成と当たる**。paddock は domain / use-case / interface / apps の
   分離が効いている一方、Topcoat は「app モジュール木＝URL 木」を前提とする。共存させると 2 つ目の
   HTTP スタックを抱える形になる（Topcoat は axum 系寄り、paddock は actix-web）。
5. **Tailwind 前提の同梱 UI が旨味にならない**。paddock の web は `web/src/styles.css` 1 枚の手書き
   ダークライブ盤で Tailwind を使っていない。同梱コンポーネント群は活かせず、新しいスタイル toolchain だけが増える。
6. **移行の実利が小さい——フロントは既に薄い**。`web/src/lib/*.ts` を確認した結果、`bets.ts` は
   API が返す `RecommendationBet`（＝Rust `build_portfolio` の出力）に UI 編集と 100 円単位ガードを
   重ねる純関数層であり、**買い方ロジックの second source にはなっていない**（ADR 0064 が警告する
   二重実装は `scripts/predict-check/live_ev.py` 側のみ）。つまり「フロントに紛れ込んだドメインロジックを
   Rust に回収できる」という移行動機は現時点の paddock には存在しない。

## 却下した代替案

- **段階移行（Topcoat と SPA を並走させ画面単位で移す）**。2 つ目の HTTP スタック・2 系統のスタイル体系・
  2 系統のテスト基盤を維持期間中ずっと抱えることになり、「一時的な修正をしない」に反する。
  0.x の breaking change を並走期間中に被り続けるのも悪い。
- **Python 部分（`scripts/predict-check/` ・`tools/mdq/`）の Rust 化を Topcoat 起点で進める**。
  これらは Web でなく Topcoat と無関係。Rust 化の是非は独立した論点であり、本 ADR で混ぜない。

## 再評価の条件

以下の **いずれか** が満たされたら再評価する。

1. **Topcoat が 1.0 に到達**（breaking change の頻度が収まる）。
2. **クライアント反応性が SPA 相当になる**（対話編集・ポーリング更新・表のソート/フィルタを
   HTMX フォールバック無しで素直に書けるようになる）。

再評価時に PoC を挟むなら、**既存 SPA・api-server を不変のまま、read-only かつ対話性がほぼ無い
新規画面 1 枚だけ**（例: 回収率レポート）を Topcoat 単体で作り、Node なしで動くか・既存レイヤを
またげるかを実測する。既存画面の書き換えから始めない。

## 影響

- **コード変更なし**。`web/`・api-server・rest-controller・`package.json`・CI いずれも不変。
- 本 ADR は決定記録のみ。CLAUDE.md・買い方ルール・本番定数への影響なし。

## 関連

- 出典: [Announcing Topcoat（tokio.rs, 2026-07-22）](https://tokio.rs/blog/2026-07-22-announcing-topcoat)、
  [tokio-rs/topcoat](https://github.com/tokio-rs/topcoat)、[crates.io: topcoat](https://crates.io/crates/topcoat)、
  [getting_started.md](https://github.com/tokio-rs/topcoat/blob/main/crates/topcoat/docs/getting_started.md)、
  [HN 議論](https://news.ycombinator.com/item?id=48952067)
- API 契約: ADR 0068（レース結果取り込みと UI 反映）、`src/interface/rest-controller/`（utoipa コードファースト＋
  `openapi.json` スナップショット検証・OpenAPI を一級成果物とする方針）。
- 買い方ロジックの二重実装警告: ADR 0064（`live_ev.py` が second source）。本 ADR は `web/src/lib/bets.ts` が
  その二重実装に**当たらない**ことを確認した記録も兼ねる。
