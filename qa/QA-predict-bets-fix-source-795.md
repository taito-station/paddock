# QA: predict_bets に買い目の固定の出所を残す（#795・2026-11-02）

#776 で記録の 3 経路（web の盤・desktop の盤・CLI の predict）が同じ固定で買い目を組むようになったが、どの固定で組んだか（出所）が `predict_bets` に残らない件の確認。
蒸留先: `knowledge/ev-kelly-bet-selection.md`（決定ログ #795）、`knowledge/product-goals.md`（REQ-D01-006 の検証手段）、`knowledge/session-write-api.md`、`knowledge/rest-api-read.md`。

## 前提（調査で確かめた事実）

- 3 経路はすべて use-case の `record_race_outcome` → repository の `save_race_outcome` を通る。
- CLI は買い目を組んだ固定の出所（`FixSource`）を手元に持つ。盤（use-case の `race_board`）は `board_fixed_selection` から出所を受け取りながら捨てていた。REST と desktop は盤の脚をそのまま記録に送り、出所を持たない。
- `predict_bets` を読む SQL・INSERT はすべて列を明示しており、列を足しても既存の SQL は壊れない。REQ-D01-006 の集計は文書にあるだけで、実装はまだない。
- #799（手精査の推奨を別枠で記録）は、同じレースに朝の固定と別枠の 2 枠を記録する（両建て）。行ごとに「どの枠の買い目か」が要る。
- ADR 0013 は馬場条件を `predict_bets` の列にせず、レース単位の別テーブル（`predict_race_conditions`）にした。

## Q1: 出所をどこに持つか

- 選択肢: **`predict_bets` に列を足す**（推奨・#799 の別枠も同じ列の値で区別できる）／レース単位の別テーブル（#799 の両建てで PK が合わなくなる）。
- 回答: **`predict_bets` に列を足す**。

## Q2: #795 より前の既存の行（出所が分からない）の扱い

- 選択肢: **NULL＝不明のまま**（推奨・埋めない。新しい行は必ず値を入れる）／'unknown' で埋める（backfill が要り、DEFAULT を付けると書き漏れを隠す）。
- 回答: **NULL＝不明のまま**。
- 補足（PR セルフレビューで判明・PO 確認 2026-11-03）: PR セルフレビューで判明した次の扱いを PO に確認し、回答「これでよい」を得た。NULL は「#795 に未対応のバイナリが書いた行」と定義する。大半は #795 より前の行だが、migrate の後も、再起動していない古い api-server・desktop・predict は NULL を書く（StaleBinary は warn で起動を続けるため）。区切りは `predict_bets.created_at` と `_sqlx_migrations.installed_on`（version 20261102000001）を比べて付ける（`created_at` は TEXT〈UTC の RFC3339〉なので `created_at::timestamptz` にキャストして比べる）。切り替え後の NULL を DB の制約（CHECK）で弾くと古いバイナリの記録が止まるので弾かない。

## Q3: 盤（web・desktop）から記録するときの出所の決め方

- 選択肢: **盤が出所を返し、記録で送る**（推奨・表示した買い目と出所が必ず一致する）／記録時にサーバで解決し直す（盤を表示した後に朝の選定ができると食い違う）。
- 回答: **盤が出所を返し、記録で送る**。

## Q4: 朝の選定の時刻を残すか・記録済みの表示に出所を出すか

- 選択肢: **時刻は残す・表示はしない**（推奨・時刻は `FixSource` が持っているので安く残せる。朝の選定は先勝ちで延期時は引き直せない）／時刻は残す・表示もする／時刻は残さない。
- 回答: **時刻は残す・表示はしない**。

## 計画の承認で確定した前提

- 列は `fix_source`（'morning' / 'recorded_axis' / 'none'）と `fix_odds_fetched_at`（'morning' のときだけ）。CHECK で組み合わせを守り、DEFAULT は付けない。
- 朝の選定の `recorded`（記録◎ を軸に強制したか）は残さない。後から `pad_predictions` と突き合わせれば作り直せる（pad を取り込み直すと再現できない）。
- 買い目があるのに出所が無ければ use-case で弾く。見送り（買い目なし）は出所を受け付けて捨てる。
