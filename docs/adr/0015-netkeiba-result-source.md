# 0015. netkeiba レース結果を results の取得源に追加し jockey/trainer を略名で正規化

## ステータス
承認済み

## コンテキスト

predict は出馬表(entry)の `jockey`/`trainer`（netkeiba `td.X a` の **略名**、例「原」「宮地」）を
キーに `jockey_stats`/`trainer_stats` を `results.jockey`/`results.trainer` と**文字列完全一致**で
join する。しかし `results` は結果 PDF 由来で次の不整合があった:

- `results.trainer`: 結果 PDF からの抽出が後付けで、live DB ではほぼ空 → trainer 項が発火しない。
  さらに PDF は**フルネーム**（「田中博康」）で、entry の netkeiba 略名（「田中博」）と一致しない。
- `results.jockey`: 旧 PDF パーサが**馬主名を連結**した汚染値（例「丸田恭介小野」「原優介奈村」）や
  破損値（「ノー」「ギン」）を含み、母数が分散・劣化（live DB で約 5%）。

netkeiba の**レース結果ページ**（`race/result.html?race_id=...`）は着順・騎手・調教師を持ち、
jockey/trainer とも entry と**同一の略名表記**で返す。OCR 不要・HTML で軽量。

## 決定

netkeiba レース結果ページを `results` の取得源として追加し、既存 `results` 行の
`jockey`/`trainer`/`finishing_position` 等を netkeiba 由来の clean な値で**更新**する。

- 新パーサ `parse_race_result`（`netkeiba-scraper`）: 結果テーブルを列クラスで解析し
  `Vec<ResultRow>`（馬番・着順/status・jockey略名・trainer略名・タイム・人気・オッズ・馬体重）を返す。
- `SqliteRepository::update_results`: `(race_id, horse_num)` 一致行のみ UPDATE。**`races` 行は触らない**
  （track_condition/weather/surface/distance 等の既存メタを保全。`save_race` は races を上書きするため使わない）。
- `fetch-results` アプリ: 既存の確定済みレースを列挙し、`build_race_ids` で netkeiba race_id を機械導出
  → 取得 → 更新。

## 理由

- entry も results も netkeiba 略名に揃うため、join が噛み合い trainer 項が live で発火し、jockey 母数の
  汚染も解消する（#82 コメントの「略名↔フルネーム正規化」課題への解）。
- netkeiba race_id は `races` の date/venue/round/day/race_num から `build_race_ids` で完全に導出でき、
  追加情報が要らない（場コードは `Venue::as_code`）。
- 結果 PDF の再 OCR（全開催で 5〜8 時間規模）に比べ、HTML 取得は ~分オーダーで現実的。
- 着順は同一レース結果のため PDF 既存値と一致し、母数の連続性が保たれる（実測でも finishing_position 不変）。

## 影響

- `results.jockey`/`results.trainer` の表記が netkeiba 略名に統一される（PDF フルネーム/汚染から変化）。
  backtest の絶対値は母数表記が変わるため過去の数値とは厳密一致しないが、entry↔results の整合が取れる。
- `results.source` は据え置き（'pdf' のまま）。データ源の混在は将来 source 区別で整理可（本 ADR ではスコープ外）。
- `update_results` は既存行の UPDATE のみで INSERT しない（新規レースは従来フローで取り込む）。
- netkeiba 結果ページに着差(margin)列が無いため margin は更新対象外（集計未使用）。
