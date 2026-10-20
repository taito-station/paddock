# 725 netkeiba の未出走馬の成績ページの観察（2026-10-20）

fetch-card が新馬戦の出走馬の近走取得を「失敗」と数えていた件（#725）の調査ログ。
`db.netkeiba.com/horse/result/<horse_id>/`（EUC-JP）を 3 秒間隔で 3 件取得し、ページの構造を見た。

## 1. 取得した 3 ページ

| horse_id | 種別 | `<title>` | `div.horse_title h1` | `table.db_h_race_results` | 「競走データがありません」 |
|---|---|---|---|---|---|
| 2024105990 | 未出走の 2 歳馬 | `クロスティーニの競走成績 \| 競走馬データ - netkeiba` | `クロスティーニ` | なし | あり（`div.contents`） |
| 2024100999 | 出走歴のある 2 歳馬 | `ナインスマイルの競走成績 \| …` | 馬名あり | あり | なし |
| 2024108224 | 存在しない ID | `の競走成績 \| …`（馬名が空） | 空 | なし | あり（`div.contents`） |

- 3 件とも HTTP は正常に返った（存在しない ID でもエラーページにはならない）。
- 未出走のページの「競走データがありません」は、`<!-- block=horse_results (d) -->` の `div.cate_bar`（見出し `<h2>…の競走成績</h2>`）の
  直後の `<div class="contents">` の本文に出る。戦績のある馬では、同じ位置に `table.db_h_race_results` が来る。
- ページ全体の `div.contents` の数は、未出走 1・存在しない ID 1・戦績のある馬 0（いずれも `div.db_main_deta` の中）。

## 2. 当時の挙動（修正前）

- `parse_horse_history` は戦績表が無いと一律に `Error::Parse("results table not found")` を返していた。
- `HorseHistoryInteractor::fetch_and_store` はこれを受けて WARN（`近走取得に失敗、スキップ`）を出し、`horses_failed` に数えていた。
- 2026-09-27 の fetch-card では、新馬の 4 レース（202606040904 / 202609040905 / 202606040905 / 202609040906）で全頭が失敗に数えられた
  （例: `近走: 0 頭（失敗 16 頭）`。`~/Library/Logs/paddock-fetch-card-20260927.log`）。そのほかの 14 レースは失敗 0 頭。
- 2026-10-18 の fetch-card ログは 7R 以降だけで、新馬戦を含んでいなかった（失敗 0 頭は判断材料にならない）。

## 3. 分かったこと

- 「戦績表が無い」だけでは、未出走・存在しない ID・HTML 構造の変化を区別できない。
- 未出走は「馬名の入った `h1`」と「競走データがありません」がそろうことで区別できる。存在しない ID は `h1` が空。
  構造が変わって戦績表のクラス名だけが変わった場合は、文言が出ないので未出走とは判定されない。
- テスト用に、2024105990 と 2024108224 のページから馬名ヘッダと競走成績ブロックだけを抜き出し、
  `src/interface/netkeiba-scraper/tests/fixtures/horse_result_unraced.html` / `horse_result_unknown.html` に置いた。
