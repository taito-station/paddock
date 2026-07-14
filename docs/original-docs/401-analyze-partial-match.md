# 原資料: #401 分析統計 API/web に #50 の部分一致・カナ正規化を露出する

> 一次資料（RO 生素材）。蒸留は QA（[QA-analyze-401](../qa/QA-analyze-401.md)）→ knowledge で行う。

## Issue #401 概要

REST の分析統計エンドポイント `GET /api/analyze/{horse,jockey,trainer}`（web の `/analyze` が使用）は
馬名・騎手名を**完全一致でしか検索できない**。表記ゆれ・部分入力で 0 件になり取りこぼす。

部分一致・カタカナ正規化のロジックは **#50（CLOSED/COMPLETED, 2026-06-09）で CLI `analyze` +
`horse_stats`/`jockey_stats` repository 層に実装済み**だが、**REST API がそれを使っておらず web から活用できない**。
#50 の既存実装を REST/web に露出させる。

### 現状の確認（2026-07-14, 経験的）
- `GET /api/analyze/horse?name=カップッチョ`（完全一致）→ 200・統計あり。
- `GET /api/analyze/horse?name=カップ`（部分）→ starts=0（未ヒット）。
- `GET /api/analyze/jockey?name=松山`（部分）→ starts=0（未ヒット）。
- 仕様書 `rest-api-read.md §4` / `web-spa.md §[4]` は「#50 に追従」と記すが、REST 側は未対応のまま（記述が stale）。

## コード調査所見（2026-07-14, worktree paddock-401 = main 4bc7d5d 起点）

### REST 現状（完全一致）
- ハンドラ `src/interface/rest-controller/src/handler/analyze.rs`
  - `analyze_horse` / `analyze_jockey` / `analyze_trainer`: 入力を `HorseName/JockeyName/TrainerName::try_from`
    で**正規化**してから `interactor.horse_stats(&name)` 等を呼ぶ。統計側は**正規化後の完全一致**。
  - クエリ型 `NameQuery { name: String }`（doc コメントに「完全一致。あいまい検索は #50」と明記）。
- ルータ `src/interface/rest-controller/src/router/analyze.rs`: `/analyze/{horse,jockey,trainer,course}`。
- スキーマ `src/interface/rest-controller/src/schema/analyze.rs`: `HorseStatsResponse` 等。
- OpenAPI `src/interface/rest-controller/src/openapi.rs`（コードファースト・utoipa）。
  スナップショット `docs/api/openapi.json`、テスト `src/apps/api-server/tests/openapi.rs`
  （再生成 `UPDATE_OPENAPI=1 cargo test -p api-server --test openapi`）。

### #50 既存資産（部分一致・normalizer 共有）
- 正規化 `src/domain/src/normalize.rs::normalize_name`（全角英数字→半角・半角カナ→全角合成・trim・冪等）。
  取り込み時（`define_string!(HorseName, normalize=normalize_name)` 等）と検索時（`*Name::try_from`）で**共有**。
- 部分一致 SQL `src/interface/rdb-gateway/src/repositories/find_matching_names.rs`:
  `SELECT DISTINCT horse_name FROM results WHERE horse_name LIKE '%'||$1||'%' ESCAPE '\' ORDER BY .. LIMIT $2`
  （中間一致・`escape_like` でワイルドカードエスケープ・名前昇順）。
- interactor `src/use-case/src/interactor/horse/stats.rs::find_horse_candidates(query, limit)`
  （jockey/trainer 版も同型）。`query` は正規化済み前提。
- use-case トレイト `NameMatchRepository`（`src/use-case/src/repository.rs:728`）は
  `Repository`（同 1089）が包含 → REST ハンドラの型境界 `R: Repository` で**候補検索を追加呼び出し可能（境界変更不要）**。

### CLI の複数ヒット挙動（踏襲元）
- `src/apps/analyze/src/bin.rs`: `find_*_candidates(query, CANDIDATE_LIMIT+1)` で上限+1件取得し打ち切り検出。
  - `[]` → 「該当なし」
  - `[one]` → その 1 件で `*_stats` を引いて表示
  - `many` → 候補一覧を提示（`CANDIDATE_LIMIT = 20`、超過は「N 件以上」）

### web 現状
- `web/src/routes/Analyze.tsx` の `NameAnalyze`: `api.GET("/api/analyze/{kind}", {query:{name}})` を直接叩き
  完全一致 stats を表示。placeholder「〜名（完全一致）」。
- 型 `web/src/api/schema.d.ts`（`npm run gen:api` = openapi-typescript で `docs/api/openapi.json` から生成）。

## 未確定（QA で解消）
- 部分一致・複数ヒットを REST でどう露出するか（新規候補エンドポイント vs 既存 stats の union 化）。
- 候補件数上限・web の導線（完全一致併用か候補一本化か）。
