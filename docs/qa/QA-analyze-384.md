# QA: #384 Analyze の検索状態保持と会場セレクト化

> 質問票+回答（[docs/qa/README.md](README.md)）。一次資料は [docs/original-docs/384-analyze.md](../original-docs/384-analyze.md)。
> 回答済みの本票を [docs/knowledge/analyze-search-and-state.md](../knowledge/analyze-search-and-state.md) に蒸留した。

## Q1: 馬/騎手/調教師の名前検索を部分一致・サジェスト化できるか（API 側の対応要否）

- 観測/根拠: `rest-api-read.md §4 分析統計`「名前あいまい検索（部分一致・カタカナ正規化, #50）は本 Issue では扱わない。完全一致で…」、`web-spa.md §[4]`「馬名・騎手名検索は #50 に追従」。分析統計 API のクエリは `name`（完全一致）で、部分一致・カタカナ正規化は横断検索側の #50（`prediction-search-api.md`「馬名の正規化」）に依存。
- 回答: **本 #384（`feat(web)`・web 完結）では扱わない。完全一致を維持し、部分一致/サジェストは #50（API 側対応）に追従する follow-up とする。**
- 反映先: `Analyze.tsx` は完全一致のまま（placeholder「（完全一致）」維持）。#50 の follow-up として PR に明記。

## Q2: タブ状態保持は URL クエリ化か state リフトアップか

- 観測/根拠: 現行は `<NameAnalyze key={kind}/>` の再マウントで入力・結果が消える。`kind` も `useState` で URL に無くリロードで初期化。既存流儀は URL 正（#379 `?date=`、`live.ts` の `parseLiveQuery`/`dashboardQueryParams`）でリロード・共有耐性を担保。ただし URL は「アクティブタブ 1 つ分」しか持てず、他タブ状態の切替保持には別途 lift が要る。
- 回答: **lift + アクティブタブを URL（ユーザー決定）。** 各タブの入力/結果は Analyze に lift して切替で保持（`key` 廃止）。加えて URL に `?kind=` とアクティブタブの検索語（name 系 `?q=`、course は `venue/distance/surface`）を反映しリロード/共有耐性も確保。
- 反映先: `web/src/routes/Analyze.tsx`（lift・`key` 廃止・`setSearchParams`）＋ `web/src/lib/analyze.ts`（URL⇔状態の純関数）。

## Q3: 会場セレクトのマスタは何を使うか

- 観測/根拠: `web/src/lib/format.ts` に `VENUE_JP`（JRA 10 場 slug→日本語）が既存。現行 course 検索は venue を free-text で slug 手入力。
- 回答: **`VENUE_JP` を流用**。`<select>` の option を JRA 場順で value=slug・label=日本語で列挙。既定は空（「開催場を選択」）。
- 反映先: `Analyze.tsx` の `CourseAnalyze` の venue 入力を `<select>` に置換。
