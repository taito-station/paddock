# CLI テストケース: fetch-card コマンド

`paddock-fetch-card` の動作確認手順。Issue #28 / 仕様書 `docs/specifications/netkeiba-datasource.md` に対応する受入観点を事前設計したもの（実装後に実施）。

netkeiba への実アクセスを伴うケースは、保存 HTML フィクスチャ or 開催日の実データで確認する。

## TC-01: race_id 直接指定での正常取り込み

| 項目 | 内容 |
|------|------|
| 前提 | 有効な 12 桁 netkeiba race_id（出馬表・単勝オッズが公開済み） |
| コマンド | `paddock-fetch-card 202605030211` |
| 期待結果 | `race_cards` に 1 件、`horse_entries` に出走頭数分、`race_odds`(bet_type=win) に頭数分が入る |
| 確認ポイント | 枠番/馬番/馬名/騎手が正しい / 単勝オッズと人気が各馬に紐づく / fetched_at がセットされる |

## TC-02: 構成要素指定での race_id 構築

| 項目 | 内容 |
|------|------|
| 前提 | 同上のレースを構成要素で指定 |
| コマンド | `paddock-fetch-card --year 2026 --venue 東京 --round 3 --day 2 --race 11` |
| 期待結果 | TC-01 と同一の race_id(`202605030211`)が組み立てられ、同じ取り込み結果になる |
| 確認ポイント | 競馬場日本語名→コード(東京=05)変換が正しい / slug 指定(`tokyo`)でも同結果 |

## TC-03: 再実行（出馬表スキップ・オッズ更新の冪等性）

| 項目 | 内容 |
|------|------|
| 前提 | TC-01 を実行済み（fetch_history に出馬表取得が記録済み） |
| コマンド | `paddock-fetch-card 202605030211` を再実行 |
| 期待結果 | 出馬表の保存はスキップ、`race_odds` は最新オッズで upsert（行数は増えない） |
| 確認ポイント | horse_entries が二重登録されない / race_odds の odds/popularity が更新され fetched_at が新しくなる |

## TC-04: --force による出馬表再取得

| 項目 | 内容 |
|------|------|
| 前提 | TC-01 を実行済み |
| コマンド | `paddock-fetch-card 202605030211 --force` |
| 期待結果 | 出馬表を再取得し race_cards/horse_entries を入れ替え、race_odds も更新 |
| 確認ポイント | 強制再取得でもエントリ重複が起きない（DELETE→再 INSERT） |

## TC-05: 単勝オッズ未確定（レース前で空欄）

| 項目 | 内容 |
|------|------|
| 前提 | 出馬表は公開済みだが単勝オッズが未確定のレース |
| コマンド | `paddock-fetch-card <該当 race_id>` |
| 期待結果 | race_cards/horse_entries は保存、race_odds(win) は空のまま（行が作られない） |
| 確認ポイント | オッズ欠損でパニックしない / 出馬表取り込みは成功する |

## TC-06: 不正な race_id

| 項目 | 内容 |
|------|------|
| 前提 | 桁数不足 or 不正な競馬場コードを指定 |
| コマンド | `paddock-fetch-card 9999` / `paddock-fetch-card --year 2026 --venue 中央 ...` |
| 期待結果 | バリデーションエラーを stderr に出力し exit code 1 |
| 確認ポイント | パニックせずエラーハンドリングされる / メッセージが原因を示す |

## TC-07: ページ取得失敗（ネットワーク/404）

| 項目 | 内容 |
|------|------|
| 前提 | 存在しない race_id、または取得不能な状況 |
| コマンド | `paddock-fetch-card <存在しない race_id>` |
| 期待結果 | 取得失敗をエラーとして報告し exit code 1 |
| 確認ポイント | パニックしない / DB が中途半端な状態にならない |
