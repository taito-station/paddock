# 一次資料: #384 Analyze の検索状態保持と会場セレクト化

> RO 生素材（[docs/docs-original/README.md](README.md)）。書き換えず、ここから qa→knowledge を蒸留する。

## 発端の Issue

原本は [#384](https://github.com/taito-station/paddock/issues/384)（**転記しない**・ADR 0074）。
本文は `gh issue view 384` で取得する。

## 調査で観測した一次情報（2026-07-14 時点・引用）

### 1. Analyze API の名前検索（doc 記述 と 実測は食い違う）
- doc（**stale**の可能性・下記実測参照）:
  - `docs/specifications/rest-api-read.md` §4 分析統計:
    > 名前あいまい検索（部分一致・カタカナ正規化, #50）は本 Issue では扱わない。完全一致でドメ…（略）
  - `docs/specifications/web-spa.md` §[4] 分析ビュー:
    > 馬名・騎手名検索は #50（部分一致・カタカナ正規化）に追従する。
  - `docs/specifications/prediction-search-api.md`「馬名の正規化（#50 流用）」に正規化の設計あり（横断検索 API 側）。
- **#50 の実状態（gh 確認）**: #50 は **CLOSED / COMPLETED（2026-06-09）**。タイトル「feat(analyze): 馬名/騎手名の
  部分一致・カタカナ正規化検索を追加」。対象は `src/apps/analyze` と `horse_stats`/`jockey_stats` repository。
- **REST の実測（2026-07-14, api-server 直叩き）**: `GET /api/analyze/horse?name=カップ`（`カップッチョ` の部分）
  → `starts=0`。`GET /api/analyze/jockey?name=松山`（`松山弘平`）→ `starts=0`。**REST は完全一致のまま**
  （#50 の正規化/部分一致は CLI/repo 止まりで REST 未露出）。→ REST/web 露出は #401 に切り出し。

### 2. 会場マスタ
- `web/src/lib/format.ts` に `VENUE_JP: Record<string,string>`（JRA 10 場、slug→日本語。sapporo/hakodate/fukushima/niigata/tokyo/nakayama/chukyo/kyoto/hanshin/kokura）が既存。

### 3. 現行 Analyze.tsx の該当挙動
- `Analyze()` は `const [kind, setKind] = useState<Kind>("horse")`（URL でない）。
- `{kind === "course" ? <CourseAnalyze/> : <NameAnalyze key={kind} kind={kind}/>}` の `key={kind}` により
  タブ切替で `NameAnalyze` が再マウントし、`input`/`name`（local useState）と結果が消える。
- `CourseAnalyze` の venue は `<input type="text" placeholder="開催場（例: nakayama）">`（slug 手入力）。
- #379 で `?date=` 読み取りと「← レース一覧へ」導線は導入済み。

## mdq 探索ログ（再現性のため）
- `scripts/mdq search --q "Analyze 分析 検索 状態 URLクエリ タブ 会場"` → web-spa §[4] / prediction-search-api。
- `scripts/mdq search --q "analyze API 名前検索 部分一致 horse jockey trainer"` → rest-api-read §4 分析統計（最上位）/ web-spa §[4] / prediction-search-api「馬名の正規化(#50)」。
- 上記チャンク本文取得で「完全一致のみ・部分一致は #50」を確認。
