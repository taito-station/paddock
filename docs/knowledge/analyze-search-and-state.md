---
status: Confirmed
kind: knowledge
sources:
  - docs/qa/QA-analyze-384.md
  - docs/qa/QA-analyze-401.md
  - docs/original-docs/384-analyze.md
  - docs/original-docs/401-analyze-partial-match.md
  - docs/specifications/web-spa.md
  - docs/specifications/rest-api-read.md
  - docs/specifications/prediction-search-api.md
distilled_from_sha: "8f8be21"
updated: "2026-07-22"
---

# 分析画面（Analyze）の検索・状態設計知

`/analyze`（`web/src/routes/Analyze.tsx`）の検索・状態設計の確定知。#384 の qa を蒸留したもの。
決定の経緯・根拠は frontmatter `sources` を参照。

## (a) 名前検索は部分一致（#50 の normalizer を REST に露出済み・#401）

- **統計本体は完全一致、候補検索は部分一致の二段構え**（CLI と同じ 0/1/多数）。#384 時点では REST 未露出で
  完全一致のみ（`カップ`/`松山` → starts=0）だったが、**#401 で候補エンドポイントとして露出**した。
  - `GET /api/analyze/{horse,jockey,trainer}/candidates?q=` → `{ names, truncated }`（中間一致 LIKE・
    名前昇順・上限 20・超過 `truncated=true`）。`q` は**取り込み時と共有の normalizer**（#50 `normalize_name`）で
    正規化。ハンドラ型境界 `R: Repository` は `NameMatchRepository` を包含するので境界変更不要。
  - `GET /api/analyze/{horse,jockey,trainer}?name=` は従来どおり**完全一致**（正規化後にドメイン値へ変換・不正 400）。
- **API 契約は「候補エンドポイント追加」を採用**（QA-analyze-401 Q1）。stats を oneOf union 化する案（B）は
  既存 stats の契約・OpenAPI・web/TS 型を壊す破壊的変更のため棄却。非破壊・既存 `find_*_candidates` 再利用・
  OpenAPI が単純、が採用理由。
- web `Analyze.tsx`（`NameAnalyze`）は「入力→候補→ 1 件は自動確定・多数は一覧クリックで確定→統計」。
  選択（`selected`）は同一コンポーネントがタブ間で使い回されるため **kind 別に親へ lift**。placeholder は「（部分一致）」。
- **URL は検索語（`?q=` 相当の name 系検索語）のみ保持し、`selected`（候補一覧からの確定名）は非永続**。
  したがって多数ヒット語で共有された URL は復元時に候補一覧までで、特定馬の統計へは深リンクしない
  （`onNameSelect` は URL を書かない意図的設計。URL に載るのはアクティブタブ 1 つ分の検索語だけ、の既存流儀と整合）。
- 仕様書 `rest-api-read.md §4`・`web-spa.md §[4]` の「#50 に追従」stale 記述は #401 で是正済み。

## (b) タブ状態保持は「URL 正 + 各タブ lift」

- タブ（馬/騎手/調教師/コース）切替で各タブの入力・結果を保持する。実装は **lift + アクティブタブを URL**:
  - 各タブの入力/確定値を `Analyze` に **lift**（`key={kind}` 再マウントを廃止）。切替で消えない。
  - URL に **`?kind=`** とアクティブタブの検索語（name 系 `?q=`、course は `venue`/`distance`/`surface`）を
    反映し、リロード・共有耐性を確保（既存流儀 = URL 正。#379 `?date=` と整合）。
  - 結果は React Query の安定 queryKey（`["analyze", kind, submitted]`）でキャッシュ再利用。
- URL は「アクティブタブ 1 つ分」しか持てないため、他タブの切替保持には lift が必須（URL 単独では不足）。

## (c) 会場は VENUE_JP の slug をセレクト

- コース検索の会場は free-text（slug 手入力）をやめ、`web/src/lib/format.ts` の **`VENUE_JP`**（JRA 10 場
  slug→日本語）を使った `<select>`（value=slug, label=日本語, 既定は空）にする。

## 変更履歴

- 2026-07-14: #384 の qa（[QA-analyze-384](../qa/QA-analyze-384.md)）を蒸留し新規作成（status=Confirmed）。
- 2026-07-14: (a) を是正。#50 は CLOSED/COMPLETED（CLI/repo に実装済）で REST が未露出、が正確な状態。
  REST/web 露出は #401 に切り出し。当初「#50 待ち」と記したのは stale な spec 記述の鵜呑みだった。
- 2026-07-14: #401 完了。(a) を「候補エンドポイントで REST 露出済み」に更新（QA-analyze-401 を蒸留）。
