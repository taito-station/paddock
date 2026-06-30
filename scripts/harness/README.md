# 学習型モデル評価ハーネス（#272 / #309）

`analyze backtest --dump-features` が吐く特徴量ダンプ TSV を入力に、内蔵モデルの
評価をリーク無しで再現（③ 忠実性サニティ）し、学習モデル（#309 Phase B）を
walk-forward で訓練して対市場評価する Python ハーネス。

依存: 忠実性サニティ（`faithfulness.py`）は**標準ライブラリのみ**。学習・評価
（`train_pl.py` / `train_gbm.py`）は numpy/scipy/scikit-learn（`requirements.txt` の venv）。

## 構成

| ファイル | 役割 | 依存 |
|---|---|---|
| `faithfulness.py` | ダンプ集計（的中率・回収率・Brier・LogLoss）と `analyze backtest` 出力との突合（③ 忠実性サニティ）。 | stdlib |
| `check_faithfulness.sh` | production 構成で backtest → dump → 突合をワンコマンド実行する忠実性ゲート。 | stdlib |
| `test_faithfulness.py` | 忠実性サニティの単体テスト（合成ダンプで手計算と一致）。 | stdlib |
| `train_pl.py` | 条件付きロジット/PL（win softmax）の walk-forward 訓練＋ baseline/市場比較（#309 Phase B）。 | numpy/scipy |
| `train_gbm.py` | 非線形 GBM 木（sklearn HGB）の walk-forward 訓練＋比較（同上）。 | scikit-learn |
| `test_train_pl.py` | 学習・予測・評価ロジックの単体テスト（合成データ）。 | numpy/scipy |
| `requirements.txt` | 学習・評価の依存ピン。`python3 -m venv .venv && .venv/bin/pip install -r requirements.txt`。 | — |

## 学習モデルの walk-forward 評価（#309 Phase B）

```sh
# 全期間ダンプ生成（production 構成・as-of＝リーク無し）→ venv → 評価
./target/debug/paddock-analyze backtest --from 2025-01-05 --to 2026-06-14 \
  --shrinkage-m 10 --win-power 1.25 --place-show-power 2.0 --blend-alpha 0.2 \
  --dump-features scripts/harness/data/dump_full.tsv
python3 -m venv scripts/harness/.venv && scripts/harness/.venv/bin/pip install -r scripts/harness/requirements.txt
scripts/harness/.venv/bin/python scripts/harness/train_pl.py  scripts/harness/data/dump_full.tsv
scripts/harness/.venv/bin/python scripts/harness/train_gbm.py scripts/harness/data/dump_full.tsv
```

**結果（OOS 3277R・棄却）**: 線形 PL・非線形 GBM のいずれも α=0.2 baseline / 市場を out-of-sample で
上回らず、市場を入れると学習モデルは市場をほぼ再現し fundamental の寄与が崩壊した（市場が過去走
fundamental を包含）。`raw_score` の学習モデル置換は見送り。詳細は
[`docs/adr/0053-learned-fundamental-model-rejected.md`](../../docs/adr/0053-learned-fundamental-model-rejected.md)。
`.venv` / `data/` は `.gitignore` 対象（再生成可能）。

## 忠実性サニティ（最重要・#309 Phase B の前提ゲート）

学習モデルを評価する前に、**Python ハーネスの集計が Rust の `analyze backtest` と同じ数値を
出すこと**を必ず確認する。設計の「最重要原則」（`docs/specifications/learned-model-harness.md`）で、
`--shrinkage-m` の付け忘れ等の設定差・ハーネスのバグを検出する回帰。

```sh
# backtest（production 構成）→ dump → 突合。一致しなければ終了コード 1。
scripts/harness/check_faithfulness.sh 2026-06-13 2026-06-14
```

`model_win/place/show` 列は backtest が校正・的中集計に使うのと同一の最終確率なので、
ハーネスは scoring を再実装せず**集計するだけ**で一致を確認できる（二重実装・ドリフトを回避）。

## テスト

```sh
cd scripts/harness && python3 -m unittest test_faithfulness -v
```

## 次（#309 Phase B）

忠実性ゲートが通ったこのハーネス上に、条件付きロジット/PL or GBM を walk-forward 学習し、
out-of-sample で α=0.2 baseline と対市場 ROI / Brier / LogLoss を比較する。採否は ADR。
