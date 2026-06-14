# predict-check — ブラインド予想→答え合わせ ハーネス

「ある開催日のレースを結果を見ずに 1R から予想し、後で結果と突き合わせて精度を測る」一連を
再実行可能にしたスクリプト群。本体 CLI（`fetch-card` / `predict`）＋ netkeiba 直パースを組み合わせる。

外部依存は `curl` と Python 標準ライブラリのみ（netkeiba は本体スクレイパが使う唯一のデータ源）。

## 背景・なぜ必要か

- `races` テーブルは成績取込済み（PDF, `source='pdf'`）のみ。予想したい当日は通常未取込。
- `predict` は `find_races_by_date` で `races` ∪ `race_cards` を見るため、**`race_cards`（出馬表）さえ
  取り込めば結果が無くても予想できる**。過去成績統計は既存 DB から引く。
- 一方 `fetch-results`（本体 CLI）は**既存 pdf レースの更新専用**で新規日を作れない。
  そこで答え合わせ用の結果は `fetch_results.py` が netkeiba 結果ページを直接パースする。

## 手順（例: 2026-06-13 の東京[05]・阪神[09]）

```bash
cd /path/to/paddock
DATE=20260613; DASH=2026-06-13; VENUES="05 09"
cargo build --release -p fetch-card -p predict

# 0. 対象 race_id を確認
python3 scripts/predict-check/list_races.py $DATE $VENUES

# 1. カード＋単勝オッズ取得（ブラインド＝結果は入れない）
for rid in $(python3 scripts/predict-check/list_races.py $DATE $VENUES | cut -f1); do
  target/release/paddock-fetch-card "$rid"   # 障害レースは exit 1（スキップ可）
done

# 1.5. 古い無効オッズ行で predict が落ちるのを回避（#114）
sqlite3 data/paddock.db "DELETE FROM race_odds WHERE odds < 1.0 AND date='$DASH';" 2>/dev/null || \
sqlite3 data/paddock.db "DELETE FROM race_odds WHERE odds < 1.0;"

# 2. 予想（スキップモードで確率表を吸い出す）。N=対象レース数ぶん "\ns\n" を流す
sqlite3 data/paddock.db "DELETE FROM predict_bets WHERE session_date='$DASH';
  DELETE FROM predict_sessions WHERE date='$DASH';"
N=$(python3 scripts/predict-check/list_races.py $DATE $VENUES | wc -l)
for i in $(seq 1 $N); do printf '\ns\n'; done \
  | target/release/paddock-predict --date $DASH --budget 10000 > /tmp/predict_out.log 2>/dev/null

# 3. 予想と結果を JSON 化
python3 scripts/predict-check/extract_preds.py /tmp/predict_out.log > /tmp/preds.json
python3 scripts/predict-check/fetch_results.py $DATE $VENUES > /tmp/results.json
sqlite3 -noheader -csv data/paddock.db \
  "SELECT race_id, combination_key, popularity, odds FROM race_odds
   WHERE bet_type='win' AND date='$DASH' ORDER BY race_id, popularity;" > /tmp/win_odds.csv

# 4. 答え合わせ
python3 scripts/predict-check/answer_check.py /tmp/preds.json /tmp/results.json /tmp/win_odds.csv
```

## スクリプト

| ファイル | 役割 |
|---|---|
| `nk.py` | netkeiba 共通ヘルパ（場コード表・race_id 列挙・結果ページパース） |
| `list_races.py` | `YYYYMMDD [場コード...]` の race_id を列挙 |
| `fetch_results.py` | 結果を取得して `results.json` 出力（答え合わせ用） |
| `extract_preds.py` | predict の stdout → `preds.json`（確率テーブル） |
| `answer_check.py` | preds × results の精度指標（本命的中/Brier/芝ダ別/回収率） |

## 注意

- 予想の主信号は**確率テーブル**。predict の推奨買い目は全正 EV 組合せを大量列挙する設計で
  ノイズが大きい（Kelly≈0%）ため、本命＝勝率最上位で評価する。
- 全レースに単勝オッズがある前提（スキップ入力 `\ns\n` の列がズレないため）。
- スコア挙動を変える改善は 144R backtest で検証してから採用する（順序ルール）。

## 実測（2026-06-13 東京・阪神 23R）

本命的中 43.5% / 本命複勝 65.2% / 勝ち馬 Top5 包含 87% / Brier 0.053 / 本命単勝回収率 101.7%。
芝中(1600-1800) が最強、ダートが最弱。詳細・改善候補は #113、predict 全停止バグは #114。
