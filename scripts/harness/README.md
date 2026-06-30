# 学習型モデル評価ハーネス（#272 / #309）

`analyze backtest --dump-features` が吐く特徴量ダンプ TSV を入力に、内蔵モデルの
評価をリーク無しで再現し、将来は学習モデル（#309 Phase B）の対市場評価に使う
Python ハーネス。依存は標準ライブラリのみ。

## 構成

| ファイル | 役割 |
|---|---|
| `faithfulness.py` | ダンプ集計（的中率・回収率・Brier・LogLoss）と `analyze backtest` 出力との突合（③ 忠実性サニティ）。 |
| `check_faithfulness.sh` | production 構成で backtest → dump → 突合をワンコマンド実行する忠実性ゲート。 |
| `test_faithfulness.py` | 集計・突合ロジックの単体テスト（合成ダンプで手計算と一致）。 |

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
