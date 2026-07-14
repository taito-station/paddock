---
status: Confirmed
kind: knowledge
sources:
  - docs/qa/QA-race-name-389.md
  - docs/original-docs/389-race-name.md
  - docs/specifications/netkeiba-datasource.md
  - docs/specifications/rest-api-read.md
distilled_from_sha: "17457a1"
updated: "2026-07-15"
---

# 出馬表の表示メタ（レース名・格付け）の取得〜表示

盤・レース一覧で重賞/特別戦を識別するための表示メタ（`race_name` #389 / `race_class` #345）の縦一本の確定知。

## レース名と格付けは別物・別ソース

- **`race_name`（#389）**: 表示用レース名。netkeiba 出馬表 `h1.RaceName`（例「安田記念」「七夕賞」「響灘特別」
  「3歳上1勝クラス」）。**グレード表記は含まない**。
- **`race_class`（#345）**: 格付け/条件スラッグ（`g1`/`g2`/`g3`/`listed`/`open`/`win3`…）。`<title>` のグレード＋
  `div.RaceData02` の条件から判定。
- 例「七夕賞(G3)」= `race_name`「七夕賞」+ `race_class` g3。**グレードは race_class から合成**（レース名の再パース不要）。
- どちらも **netkeiba 経路のみ**が埋める best-effort。取得失敗・PDF 経路・過去分は `None`（カード保存は止めない）。

## 縦一本のチェーン（post_time/#235・race_class/#345 と同型）

- scraper `parse/card.rs::extract_race_name`（`h1.RaceName`・trim・空は None）→ `FetchedCard.race_name`
  → `interactor/card/ingest.rs` → domain `RaceCard.race_name: Option<String>`。
- DB: `race_cards.race_name TEXT`（migration `20260708000003`・自由テキストで CHECK なし）。
  `save_race_card` は `COALESCE(excluded.race_name, race_cards.race_name)` で **PDF 経路 None が netkeiba 値を消さない**。
  `find_race_card` の SELECT に race_name（自由テキストなので往復バリデーション不要）。過去分の埋め戻し不要（NULL 許容）。

## API 露出（3 レスポンス）

- **盤ヘッダ** `GET /api/races/{id}/board` = `RaceBoardResponse`（RaceCardResponse ではない）に
  `race_name` + `race_class`（グレード表示のため両方）。use-case `RaceBoard` が card から抽出。
- **一覧** `GET /api/races` = `RaceSummary` に `race_name` のみ（識別が主目的。グレードは follow-up 余地）。
  一覧は `find_race_names_by_date`（`race_id → race_name` マップ、race_cards 由来・`find_post_times_by_date` と同方針）で
  一括引き当て → `RaceSummary::new(r, &post_times, &race_names)`。NULL 行はマップに含めない。
- **出馬表単体** `GET /api/races/{id}` = `RaceCardResponse` に `race_name` + `race_class`（完全性）。
- OpenAPI はコードファースト。race_name/race_class 追加時は `docs/api/openapi.json` 再生成必須。

## web 表示

- **盤ヘッダ**: `format.ts::raceTitle(race_name, race_class)` で合成。**重賞・L（g1/g2/g3/listed）のみ `名(グレード)`**、
  open/win*/maiden 等の条件クラスは race_name 自体が自己完結（「響灘特別」「3歳上1勝クラス」）のためグレード付与なし
  （`RACE_CLASS_JP` はグレードのみ持つ）。ヘッダ = 「会場 R 馬場距離 raceTitle」。race_name 無ければ従来の条件表示のみ。
- **一覧**: 新列を足さず会場R セルの下に `.race-name-sub`（block・muted・小）で `race_name` があるときだけ副次表示。
  ライブ表（`DashboardRowView`）と静的 fallback 表（`RaceList` StaticRow）の両方。列・ソート構成は不変（非侵襲）。
