# QA: #401 分析統計 API/web への部分一致・カナ正規化の露出

> 質問票+回答（[docs/qa/README.md](README.md)）。一次資料は [docs/original-docs/401-analyze-partial-match.md](../original-docs/401-analyze-partial-match.md)。
> 回答済みの本票は実装後 [docs/knowledge/analyze-search-and-state.md](../knowledge/analyze-search-and-state.md) に蒸留する。

## Q1: 部分一致・複数ヒットを REST でどう露出するか（API 契約）

- 観測/根拠:
  - REST `analyze_{horse,jockey,trainer}` は `*Name::try_from`（正規化）→ `*_stats`（**正規化後の完全一致**）。
  - 部分一致は `interactor.find_*_candidates(query, limit)`（中間一致 LIKE・normalizer 共有）が**既存**。
    CLI はこれで「0件→なし / 1件→統計 / 多数→候補一覧（上限 20・超過は N 件以上）」の**二段構え**を実装済み。
  - ハンドラ型境界 `R: Repository` は `NameMatchRepository` を包含 → 候補検索の追加呼び出しに**境界変更不要**。
  - 選択肢:
    - **A. 候補エンドポイント追加**: `GET /api/analyze/{horse,jockey,trainer}/candidates?q=` を新設し
      `{ names, truncated }` を返す。既存 stats（完全一致）は据え置き。web は「入力→候補→確定→stats」。
    - **B. stats を union 化**: `?name=` を部分一致化し 1件→統計 / 多数→候補 / 0件→404 を **oneOf** で返す。
- 回答: **A（候補エンドポイント追加）を採用（ユーザー決定 2026-07-14）。**
  - 理由: (1) **非破壊** — 既存 stats のレスポンス契約・OpenAPI スナップショット・web の統計表示・TS 型を壊さない。
    B は stats を oneOf union に変える破壊的変更で web/型に分岐が波及する。
    (2) **既存資産の素直な再利用** — `find_*_candidates` と CLI の二段構えをそのまま REST に移すだけ。
    (3) **OpenAPI が単純** — 候補は `{ names: string[], truncated: bool }` の 1 スキーマ。oneOf を持ち込まない。
    (4) **シンプル第一 / CLI と一貫** — CLI・REST で「候補を引いてから統計を引く」導線が揃う。
- 反映先: schema `AnalyzeCandidatesResponse`、handler `analyze_*_candidates`、router `/candidates`、
  openapi.rs（paths/components）、`docs/api/openapi.json` 再生成。

## Q2: 候補件数の上限は

- 観測/根拠: CLI は `CANDIDATE_LIMIT = 20`（上限+1件取得で打ち切り検出、超過は「N 件以上」表示）。
- 回答: **REST も 20 で揃える**。ハンドラ内定数として持ち、`find_*_candidates(q, LIMIT+1)` で取得 →
  `truncated = 件数 > LIMIT` を立てて 20 件に切り詰めて返す。CLI と同じ打ち切りセマンティクス。

## Q3: web の導線（完全一致併用か候補一本化か）

- 観測/根拠: 現行 `NameAnalyze` は submit で stats を直接叩き完全一致表示。placeholder「（完全一致）」。
- 回答: **候補一本化（CLI と同じ 0/1/多数）**。submit で候補エンドポイントを叩き:
  - 0 件 → 「該当なし」。
  - 1 件 → その名前で stats を自動取得して表示（完全一致 stats エンドポイント再利用）。
  - 多数 → 候補名をクリック可能な一覧で提示（`truncated` 時は「絞り込んでください」を添える）。クリックで stats。
  - placeholder は「〜名（部分一致）」に是正。React Query の queryKey は候補・stats で分離。
- 反映先: `web/src/routes/Analyze.tsx`（`NameAnalyze` を候補導線に改修）、`web/src/api/schema.d.ts` 再生成。

## Q4: stale な spec 記述の是正

- 観測/根拠: `rest-api-read.md §4` / `web-spa.md §[4]` の「#50 に追従」は、#50 が CLI/repo 完了・REST 未露出の
  状態を正しく表さない（#384 knowledge で一度是正済みだが、露出後は「#401 で REST 露出済み」に更新が要る）。
- 回答: **本 issue で両 spec を「#401 で REST に候補エンドポイントとして露出済み」に更新**し、knowledge (a) も追従。
