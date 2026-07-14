# QA: #389 レース名（重賞・特別戦名）の取得・保存・表示

> 質問票+回答（[docs/qa/README.md](README.md)）。一次資料は [docs/original-docs/389-race-name.md](../original-docs/389-race-name.md)。
> 回答済みの本票は [docs/knowledge/race-card-display-metadata.md](../knowledge/race-card-display-metadata.md) に蒸留した。

## Q1: レース名の抽出元とグレードの扱い

- 観測/根拠: `h1.RaceName` はレース名のみ（「安田記念」）でグレードを含まない。グレードは `<title>`＋
  `RaceData02` から判定した既存 `race_class`（#345）にある。例「七夕賞(G3)」= レース名 + グレード。
- 回答: **レース名は `h1.RaceName` から取得（`race_name: Option<String>`）。グレードは既存 `race_class` を露出して
  web 側で合成**（新規パースしない）。取得失敗・空は `None`（best-effort・カード保存は止めない。post_time/race_class と同方針）。
- 反映先: scraper `extract_race_name`、domain `RaceCard.race_name`、`FetchedCard.race_name`、ingest。

## Q2: どの API レスポンスに露出するか

- 観測/根拠: 盤ヘッダは `RaceBoardResponse`（`/board`）由来で RaceCardResponse ではない。一覧は `RaceSummary`。
- 回答: **盤 = `RaceBoardResponse` に `race_name` + `race_class`（グレード表示のため両方）。一覧 = `RaceSummary` に
  `race_name` のみ**（識別が主目的。グレードは follow-up 余地）。出馬表単体 `RaceCardResponse` も `race_name` +
  `race_class` を露出（完全性。RaceCard が両方保持済みで低コスト）。
- 反映先: schema `race.rs`（3 レスポンス）、`RaceBoard`（use-case）に race_name/race_class 追加、list handler で
  `race_names_by_date` マップ照合、OpenAPI 再生成。

## Q3: 一覧のレース名保存メタはどう引き当てるか

- 観測/根拠: 一覧は `find_races_by_date`（`Race`・race_name なし）＋ `find_post_times_by_date`（マップ）で post_time を照合。
- 回答: **`find_race_names_by_date`（`race_id → race_name`、race_cards 由来、post_times と同方針）を新設**し
  `RaceSummary::new(r, &post_times, &race_names)` で照合。race_name NULL 行はマップに含めない（PDF 経路・過去分・正常系）。

## Q4: 一覧の UI 表示方式

- 観測/根拠: 一覧は静的 fallback 表と密なライブ表（`DashboardRowView`）の 2 系。ライブ表は列・ソートが密。
- 回答: **新列を足さず、会場R セルの下に副次表示**（`.race-name-sub`＝block・muted・小）。ライブ表の「レース」セルと
  静的表の会場セルの両方に、`race_name` があるときだけ出す。列/ソート構成を壊さず非侵襲（発見文脈のライブ監視に効く）。
- 反映先: `DashboardRowView.tsx`、`RaceList.tsx`（StaticRow）、`styles.css`（`.race-name-sub`・:root トークンのみ）。

## Q5: 盤ヘッダの表示形とグレード付与ルール

- 観測/根拠: 盤ヘッダは「会場 R 馬場距離」。例は「七夕賞(G3)」「響灘特別」。
- 回答: **`format.ts::raceTitle(race_name, race_class)` で合成。重賞・L（g1/g2/g3/listed）のみ `名(グレード)`、
  open/win*/maiden 等の条件クラスは `race_name` 自体が自己完結（「響灘特別」「3歳上1勝クラス」）なのでグレード付与なし**。
  ヘッダは「会場 R 馬場距離 raceTitle」。race_name 無ければ従来の条件表示のみ。
- 反映先: `format.ts`（`RACE_CLASS_JP`＝グレードのみ・`raceTitle`）、`RaceBoard.tsx` ヘッダ。

## Q6: DB スキーマ・非破壊性

- 回答: **migration `20260708000003_add_race_cards_race_name`（`ADD COLUMN IF NOT EXISTS race_name TEXT`・自由テキストで
  CHECK なし）**。`save_race_card` は `COALESCE(excluded.race_name, race_cards.race_name)` で PDF 経路 None が netkeiba 値を
  消さない（race_class と同方針）。過去分の埋め戻しは不要（NULL 許容）。
