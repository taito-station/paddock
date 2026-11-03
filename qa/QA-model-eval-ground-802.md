# QA: モデル精度の評価の土俵（#802・2026-11-02）

#801（モデルは当たることで評価する）を受けて、採否判定の土俵を作るときに確定した点。
回答は 2026-11-02 の会話でユーザーが選んだもの（質問票は AskUserQuestion で実施）。
一次資料: `docs-original/802-model-eval-baseline.md`。蒸留先: `knowledge/backtest.md`（評価プロトコル・決定ログ #802）、`knowledge/product-goals.md`（REQ-D01-009〜011・決定ログ #802）。

## Q1: 買い方層の指標（純モデルが市場より高く見た馬が、市場の見立てより実際に来ているか）を #802 に含めるか

- 背景: 起票時の #802 は、モデル精度の指標と並べて「乖離帯ごとの実現率 − implied」も同じコマンドで出す案だった。
  一方で #801 の整理で「モデルに関しては買い方の議論をするな」と指示があった。
- 回答: **#803（買い方の軸の構造の比較）に移す**。#802 はモデル精度の土俵だけに絞る。

## Q2: 「例年そのレースでは内枠が強い」の特徴量をどう扱うか

- 背景: 今の `course_gate_rate` はコース（場 × 距離 × 芝ダ）単位の枠グループの複勝率で、レース名・時期単位の傾向は無い。
  過去レースの `races` に race_name が無く（netkeiba 経路の `race_cards.race_name` と `horse_past_runs.race_name` にしか無い）、取り込み設計が先に要る。
- 回答: **別 issue（#807）に分ける**。#802 は土俵を作るところまで。土俵ができてから、最初の検証対象として #807 で測る。

## Q3: 評価の実装場所

- 背景: Rust の `paddock-analyze backtest` が as-of で的中率・Brier・LogLoss・reliability を出し、`--dump-features` の TSV を
  Python `scripts/predict-check/prob_eval.py` が読んで較正（CORP）と対市場 ΔR² を出している。足りないのは、市場との的中率比較・
  上位 N 包含率・日単位 bootstrap・JSON 出力。
- 回答: **Python で `--dump-features` の TSV を評価する**（`scripts/predict-check/model_eval.py`）。`prob_eval.py` の `load_dump` /
  `market_probs` を流用し、Rust は変更しない。ツール層は Python でよいという既存方針に沿う。

## Q4: 採否の基準（Plan で提示・承認）

- 回答: 候補 − 基準 の主指標（◎ 複勝的中率・◎ 単勝的中率・top5 包含 3 頭率・win LogLoss）を日クラスタ bootstrap の 95% CI で見る。
  **◎ 複勝的中率の Δ の CI が正、または win LogLoss の Δ の CI が負**で、かつ悪化側で 0 を跨がない主指標が無ければ「採用候補」。
  悪化側で 0 を跨がない主指標が 1 つでもあれば「悪化」。それ以外は「判定不能」。
  eval 窓は凍結窓（2026-01-01〜08-31）。窓・bootstrap 回数・seed は実行前に決め、結果を見てから動かさない。
