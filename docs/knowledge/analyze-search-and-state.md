---
status: Confirmed
kind: knowledge
sources:
  - docs/qa/QA-analyze-384.md
  - docs/original-docs/384-analyze.md
  - docs/specifications/web-spa.md
  - docs/specifications/rest-api-read.md
  - docs/specifications/prediction-search-api.md
distilled_from_sha: "27012d2"
updated: "2026-07-14"
---

# 分析画面（Analyze）の検索・状態設計知

`/analyze`（`web/src/routes/Analyze.tsx`）の検索・状態設計の確定知。#384 の qa を蒸留したもの。
決定の経緯・根拠は frontmatter `sources` を参照。

## (a) 名前検索は完全一致（REST 未露出。露出は #401）

- REST `GET /api/analyze/{horse,jockey,trainer}` の `name` は**完全一致**（2026-07-14 経験的に確認:
  部分入力 `カップ`/`松山` は starts=0）。web はこの REST を叩くため完全一致。
- 部分一致・カタカナ正規化ロジック自体は **#50（CLOSED/COMPLETED）で CLI `analyze` +
  `horse_stats`/`jockey_stats` repository 層に実装済み**。だが **REST API がそれを使っておらず未露出**。
  仕様書 `rest-api-read.md §4`・`web-spa.md §[4]` の「#50 に追従」は stale。
- したがって web 完結の #384 では**完全一致を維持**。REST/web への露出は新規 issue **#401**（#50 の既存
  normalizer を再利用）で行う。

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
