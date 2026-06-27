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
#  race_odds に date 列は無いので paddock race_id（list_races の 3 列目）で当該開催だけ消す。
#  無条件 `WHERE odds < 1.0` は他開催を巻き込むため使わない。
for pad in $(python3 scripts/predict-check/list_races.py $DATE $VENUES | cut -f3); do
  sqlite3 data/paddock.db "DELETE FROM race_odds WHERE odds < 1.0 AND race_id='$pad';"
done

# 2. 予想（スキップモードで確率表を吸い出す）。N=対象レース数ぶん "\ns\n" を流す
sqlite3 data/paddock.db "DELETE FROM predict_bets WHERE session_date='$DASH';
  DELETE FROM predict_sessions WHERE date='$DASH';"
N=$(python3 scripts/predict-check/list_races.py $DATE $VENUES | wc -l)
for i in $(seq 1 $N); do printf '\ns\n'; done \
  | target/release/paddock-predict --date $DASH --budget 10000 > /tmp/predict_out.log 2>/dev/null

# 3. 予想と結果を JSON 化
python3 scripts/predict-check/extract_preds.py /tmp/predict_out.log > /tmp/preds.json
python3 scripts/predict-check/fetch_results.py $DATE $VENUES > /tmp/results.json
# 単勝オッズ CSV（ROI/人気比較用）。race_odds は paddock race_id で引く（date 列は無い）
: > /tmp/win_odds.csv
for pad in $(python3 scripts/predict-check/list_races.py $DATE $VENUES | cut -f3); do
  sqlite3 -noheader -csv data/paddock.db \
    "SELECT race_id, combination_key, popularity, odds FROM race_odds
     WHERE bet_type='win' AND race_id='$pad' ORDER BY popularity;" >> /tmp/win_odds.csv
done

# 4. 答え合わせ
python3 scripts/predict-check/answer_check.py /tmp/preds.json /tmp/results.json /tmp/win_odds.csv
```

## 買い方（馬券構成）別の回収率評価（#122）

「軸を決めて相手に流す」買い方は、単勝のみより回収率が大きく変わる（#122）。確定配当を取得し、
本命を軸にした 単勝のみ / 馬連＋ワイド流し / ＋三連複流し の戦略別回収率を比較する。
上記 3 の `preds.json` を流用し、確定配当を追加取得して評価する。

```bash
# 5. 確定配当（馬連/ワイド/三連複等）を取得
python3 scripts/predict-check/fetch_payouts.py $DATE $VENUES > /tmp/payouts.json

# 6. 戦略別回収率（予想本命を軸に）。相手頭数・予算配分を振って感度も見られる
python3 scripts/predict-check/strategy_eval.py /tmp/preds.json /tmp/payouts.json --budget 5000 --partners 5
python3 scripts/predict-check/strategy_eval.py /tmp/preds.json /tmp/payouts.json --budget 5000 --partners 3,5,7
python3 scripts/predict-check/strategy_eval.py /tmp/preds.json /tmp/payouts.json --budget 5000 --alloc 4,2,1
# 人気馬を軸にする場合（win_odds.csv が必要）
python3 scripts/predict-check/strategy_eval.py /tmp/preds.json /tmp/payouts.json --axis market --win-odds /tmp/win_odds.csv
```

出力例（戦略別 回収率・収支）:

```
戦略                               回収率          収支         賭け計         払戻計
本命単勝のみ                         51.7%     -87,000     180,000      93,000
本命軸 馬連+ワイド流し                   68.7%     -56,300     180,000     123,700
本命軸 馬連+ワイド+三連複流し               68.3%     -45,670     144,000      98,330
```

精算は `payouts.json`（確定した的中組のみが入る）への**組番一致**で判定する。確定配当そのものが
「どの組が当たったか」を持つため、着順から的中を再導出しない（同着・複数本ワイドも自然対応）。

`消化率` は予算上限（`budget × 評価レース数`）に対する実際の賭け額の割合。100 円単位の端数切り捨てで
予算を使い切らない戦略（点数が多く 1 点あたりが小さい三連複流し等）はここが 100% 未満になる。

## スクリプト

| ファイル | 役割 |
|---|---|
| `nk.py` | netkeiba 共通ヘルパ（場コード表・race_id 列挙・結果/確定配当パース） |
| `list_races.py` | `YYYYMMDD [場コード...]` の race_id を列挙 |
| `upcoming_races.py` | 発走時刻で「これから発走する直近レース」だけを列挙（発走済み除外＋発走 N 分以内, #197） |
| `fetch_results.py` | 結果を取得して `results.json` 出力（答え合わせ用） |
| `fetch_payouts.py` | 確定配当を取得して `payouts.json` 出力（戦略評価用, #122） |
| `extract_preds.py` | predict の stdout → `preds.json`（確率テーブル） |
| `answer_check.py` | preds × results の精度指標（本命的中/Brier/芝ダ別/回収率） |
| `strategy_eval.py` | preds × payouts の買い方別回収率（軸流し・予算配分・100 円単位, #122） |
| `konsen_backtest.py` | 混戦判定の閾値バックテスト（¥5,000・確率重み配分・3連複ボックス, #180） |
| `formation_backtest.py` | 上位近接時の2軸フォーメーション バックテスト（baseline vs union2/pair2・θ 掃引, #241） |
| `umaren_backtest.py` | 馬連特化（馬連 EV≥θ）買い目のバックテスト（frequency 比較・cap/flat/weighted 掃引, #250） |
| `fetch_wide.py` | netkeiba ライブ（発走前）ワイドオッズ取得（type=5, fetch-card 未対応の補完, #187） |
| `live_ev.py` | 当日・発走前オッズの全3券種 ROI（期待回収率）評価＋ +EV レースの買い目伝票 |
| `refresh_ev.sh` | ライブ EV のオーケストレータ（fetch-card→DB→ワイド→predict→`live_ev.py`） |

## 注意

- 予想の主信号は**確率テーブル**。predict の推奨買い目は全正 EV 組合せを大量列挙する設計で
  ノイズが大きい（Kelly≈0%）ため、本命＝勝率最上位で評価する。
- 全レースに単勝オッズがある前提（スキップ入力 `\ns\n` の列がズレないため）。
- スコア挙動を変える改善は 144R backtest で検証してから採用する（順序ルール）。

## 実測（2026-06-13 東京・阪神 23R）

本命的中 43.5% / 本命複勝 65.2% / 勝ち馬 Top5 包含 87% / Brier 0.053 / 本命単勝回収率 101.7%。
芝中(1600-1800) が最強、ダートが最弱。詳細・改善候補は #113、predict 全停止バグは #114。

## 混戦判定の閾値バックテスト（#180）

`konsen_backtest.py` は確定買い方（¥5,000・ワイド/馬連/3連複を model 確率で重み付け配分・
混戦は3連複ボックス追加）を複数開催日の全レースへ機械適用し、**混戦の発動条件**を切り替えて
回収率を比較する。入力は predict-check ハーネスの中間生成物（races TSV / win_odds TSV /
predict 出力 / netkeiba result.html）。

```bash
python3 scripts/predict-check/konsen_backtest.py \
    --races /tmp/bt_races.tsv --winodds /tmp/bt_winodds.tsv \
    --pred-dir /tmp --results-dir /tmp --odds-grid 3.0,3.5,4.0
```

**結論（2026-05/06 の 71R で検証, #180）**: 「◎単勝オッズが割れている（弱い1番人気）なら
混戦扱い」案は **不採用**。`box-off 84.0% ≧ baseline(band≥4) 83.9% ＞ オッズ条件併用 79.5〜81.5%` で、
どの閾値でも回収を悪化させた（回収率の分母を実消化額にした公平な会計でも同じ）。band=2 では
ボックスが組成不能（3頭未満）で予算が余る。既存 band≥4 ボックスの寄与も中立。詳細は #180 のコメント参照。

## ライブ EV 監視（当日・発走前オッズ）

ブラインド予想→事後答え合わせ（上記）とは別に、**開催当日に発走前の最新オッズで「いま張る価値が
あるレース」を見つける**ための一連。「高的中・低配当」を避け、**全3券種 ROI（期待回収率）が
+EV（≥100%）のレースだけ張る**方針（買い方は上記戦略評価と同一の確率重み配分・混戦ボックス）。

> このフローの DB アクセスは **Postgres**（`PADDOCK_DB_URL`、本体が SQLite→Postgres 移行済み）。
> 上のブラインド予想手順にある `sqlite3 data/paddock.db ...` は移行前の記述で、現行は Postgres。

```bash
# 当日 6-12R を最新オッズで再取得し ROI ランキング＋ +EV レースの買い目伝票を出す。
# 15 分間隔等で回し、ROI>=100% かつ未走のレースが出たら張る。
scripts/predict-check/refresh_ev.sh 2026-06-20 6 12 5000
#                                    └日付      │ │ └1レース予算(円)
#                                              └─┴ R 範囲（first last）

# 発走時刻ウィンドウ絞り込み（#197）: LIVE_WINDOW_MIN を付けると、netkeiba 発走時刻で
# 「これから発走する かつ 発走まで N 分以内」のレースだけに絞る。朝の早い時間帯に全レースを
# 叩いてオッズが動かないのに netkeiba を過剰アクセスする無駄を防ぐ（feedback_jra_fetch_pacing）。
# 15 分ループから回すときはこれを付け、第1R発走 1 時間前まで実質間引き・本格化後に本気稼働する。
LIVE_WINDOW_MIN=60 scripts/predict-check/refresh_ev.sh 2026-06-20 1 12 5000
```

`refresh_ev.sh` の処理:

1. `fetch-card --force`（netkeiba 最新オッズ → Postgres `race_odds`）
2. Postgres から馬・単勝・馬連・3連複オッズを TSV 化（`$WORKDIR`、既定 `$TMPDIR/paddock-live-ev`）
3. `fetch_wide.py`（netkeiba type=5）でワイドを取得（fetch-card 未対応の補完, #187）
4. `analyze predict --blend-alpha 0.2`（最新オッズ込みの model 勝率）で確率テーブル生成
   （α は本番モデルと同じ 0.2＝ADR 0034。実験時は `LIVE_BLEND_ALPHA` で上書き可）
5. `live_ev.py` が Plackett-Luce（model 勝率→着順確率）× 実オッズで全3券種 ROI を算出

`live_ev.py` は中間 TSV を直接渡しても単独実行できる（再計算のみ・取得をスキップ）:

```bash
python3 scripts/predict-check/live_ev.py \
    --pred $WORKDIR/pred.txt --meta $WORKDIR/meta.tsv --horses $WORKDIR/horses.tsv \
    --exotic $WORKDIR/exotic.tsv --wide $WORKDIR/wide.tsv --budget 5000 --slip
```

**実測（2026-06-20 / 21R）**: +EV は函館12R（◎④ 1.7倍 model35% ROI≈125%）のみ。¥10,000 に増額し
④→⑤→⑦ 本命決着で回収 222%。−EV で見送った断然人気（東京10R 1.7倍 ROI80% 等）は全て不的中 or
薄配当で、ROI 基準の取捨が利益に直結した。

> ⚠️ ライブ監視では **最新サイクルの判定のみが有効**。オッズで ◎ も +EV 判定も入れ替わるため、
> 前サイクル/朝のランキングは無効化して扱う。
