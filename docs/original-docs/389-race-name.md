# 原資料: #389 盤・レース一覧にレース名（重賞・特別戦名）を表示する

> 一次資料（RO 生素材）。蒸留は QA（[QA-race-name-389](../qa/QA-race-name-389.md)）→ knowledge で行う。

## 発端の Issue

原本は [#389](https://github.com/taito-station/paddock/issues/389)（**転記しない**・ADR 0074）。
本文は `gh issue view 389` で取得する。

## コード調査所見（2026-07-14, worktree paddock-389 = main 17457a1 起点）

### レース名の在り処（netkeiba 出馬表 HTML）
- `h1.RaceName` にレース名（例「安田記念」「七夕賞」「響灘特別」「3歳上1勝クラス」）。**グレード表記は含まない**。
- グレード（(G1) 等）は `<title>` にあり、条件は `div.RaceData02`。両者から `race_class`（#345）を判定済み。
  ＝例の「七夕賞(G3)」の「(G3)」は既存 `race_class` から合成できる（新規パース不要）。

### 縦一本のチェーン（既存パターン）
- domain `RaceCard`（`src/domain/src/race_card/mod.rs`）に post_time/#235・race_class/#345 が Option で載る前例。
- scraper `parse/card.rs::parse_card` → `FetchedCard`（use-case）→ `interactor/card/ingest.rs` で `RaceCard`。
- DB: `race_cards` テーブル（migration `20260708000001` で race_class 追加＝ALTER ADD COLUMN + COALESCE upsert が前例）。
  `save_race_card`（COALESCE で netkeiba 値を PDF None で消さない）／`find_race_card`（SELECT）。
- 一覧の付随メタは `find_post_times_by_date`（race_id→post_time マップ, #391）で一括引き当て、
  `RaceSummary::new(r, &post_times)` で照合する前例（レース名も同手で通せる）。
- API 露出先: 盤ヘッダは `GET /api/races/{id}/board` = **RaceBoardResponse**（RaceCardResponse ではない）。
  一覧は `RaceSummary`。出馬表単体は `RaceCardResponse`。use-case `RaceBoard` は card から venue/surface/... を抽出済み。

## 未確定（QA で解消）
- グレード「(G3)」を出すか（race_class 露出の要否）／どの層に露出するか。
- 一覧にレース名をどう出すか（新列 vs 副次表示）。グレードも一覧に出すか。
- レース名の取得失敗時の扱い（best-effort か必須か）。
