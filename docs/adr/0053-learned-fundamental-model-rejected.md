# 0053. 学習型 fundamental モデル（条件付きロジット/PL・非線形 GBM）への raw_score 置換の棄却（#309 / #272 Phase B）

## ステータス

棄却（production の `raw_score`＋α=0.2 市場ブレンドを維持。学習モデルへの置換は見送り。ADR 0027/0052 の再確認）

## コンテキスト

#309（#272 配下）は、手作りの線形レート加重平均である `raw_score`（`src/domain/src/prediction/scoring.rs`）を
データ駆動の学習ランカーへ置換し、「市場との意味ある食い違い＝エッジ」を出せるかを検証する issue。残る本質
レバーは（ADR 0027 の通りデータ量でなく）**モデルクラス**との仮説で、まず 1 手法を小さく検証し、勝てなければ
素直に棄却 ADR を残す方針だった。

前提として #272 Phase A で忠実性ハーネス（`analyze backtest --dump-features` の as-of ダンプ＋Python 評価が
backtest と一致することを担保、PR #310/#311/#312）を整備済み。本 ADR はそのダンプを入力に、学習モデルを
walk-forward で訓練し α=0.2 baseline・純市場と out-of-sample 比較した結果を記録する。

### リーク防止

特徴量は `analyze backtest`（help: "Reproduces probability estimation with as-of stats (no leakage)"）の
as-of ダンプ（統計は予測対象日 `< D`）。さらに日付分割で**訓練は予測窓より前の日付のみ**（expanding window）に
限定する。production 構成（m=10 / win_power=1.25 / place_show_power=2.0 / α=0.2）で全期間ダンプを生成した。

## 検証（OOS 3277R / 訓練 2025-01〜・評価 2025-07〜2026-06・月次 expanding walk-forward）

全期間ダンプ（4891R / 68,148 出走馬 / 2025-01-05〜2026-06-14）を、`date < cutoff` で訓練し前方 1 か月を
予測する月次ローリングで OOS 予測を得た（構造的にリーク無し）。2 手法を実装し、いずれも基礎特徴量のみと
市場併載の 2 変種で baseline・純市場と比較した:

- **PL（条件付きロジット）**: レース内 softmax `P(i 1着)=softmax(β·x_i)` を winner の条件付き対数尤度で
  L2 正則化付き当てはめ（McFadden）。特徴量 = factor 勝率6＋signal3（標準化、欠落は訓練 fold 平均/中立補完）。
- **HGB（非線形 GBM 木）**: ヒストグラム勾配ブースティング（sklearn `HistGradientBoostingClassifier`）。
  特徴量 = 上記9＋factor 出走数 starts6（rate×starts 交互作用を木が使える、欠落は NaN のまま）。レース内正規化で
  win 確率化。LightGBM は libomp（OpenMP 共有ライブラリ）を要求し未導入環境でロードできないため、libomp 不要・
  NaN ネイティブで同じヒストグラム勾配ブースティング系の sklearn HGB を採用した。なお HGB は PL のような
  レース条件付き softmax ではなく、出走馬を pooled な per-horse 二値分類（is_winner）で学習し**学習後にレース内
  正規化**する近似（fundamental の marginal 寄与を非線形・交互作用込みで見る目的には十分）。

| モデル | 単勝 Brier | 単勝 LogLoss | flat ROI |
|---|---|---|---|
| 純市場（implied） | **0.0551** | **0.1975** | 74.7% |
| baseline（α=0.2・現行） | 0.0552 | 0.1981 | 74.7% |
| PL 基礎（fund のみ） | 0.0614 | 0.2343 | 70.0% |
| PL 市場あり（fund+mkt） | 0.0552 | 0.1977 | 75.7% |
| HGB 基礎（fund のみ） | 0.0610 | 0.2317 | 76.7% |
| HGB 市場あり（fund+mkt） | 0.0554 | 0.1988 | 74.6% |

（Brier/LogLoss は小さいほど良い・全出走馬を独立 Bernoulli とした **per-horse** スコア（race-level の
`-log p_winner` ではない）で全モデル共通母数のため比較は公平。flat ROI は「トップ選好馬の単勝 100 円」固定の
**総払戻倍率／賭けレース数＝粗の払戻率**（net ROI ではない）。PL は L2=1/10/100 で同結論＝正則化に頑健。）

**観察**:

- **市場を入れると学習モデルは「市場をほぼ再現し fundamental を無視」する。** PL 市場ありの係数は
  `log_market_implied = +1.04`（≈市場そのまま）に対し**全 fundamental 係数が ±0.05 未満に崩壊**（最大でも
  jockey_recent_form の +0.043。基礎のみでは jockey_surface_win +0.34 等が効くのと対照的）（`train_pl.py`
  が main で当てはめ係数を出力する。`scripts/harness/.venv/bin/python scripts/harness/train_pl.py <dump>` の
  「学習係数」節で再現可能）。HGB 市場ありも Brier 0.0554・LogLoss 0.1988 と純市場（0.0551/0.1975）・
  baseline（0.0552/0.1981）に並ぶだけで改善しない。**線形・非線形いずれでも市場が fundamental シグナルを
  包含**しており、市場に対する marginal な情報を過去走 fundamental から取り出せていない。
  - 係数のスケール注記: fundamental は標準化済み（z-score）係数、`log_market_implied` は log-implied 生値の
    係数で、直接の絶対値比較はスケールが異なる。`β_market ≈ 1` は `softmax(log implied) = implied`＝**市場の
    完全再現**を意味する意図的設計で、その上で fundamental 係数が ±0 へ落ちる（市場再現で十分＝fundamental
    不要）という**定性**が結論の核。
- **fundamental のみのモデルは市場に校正で劣る。** PL 基礎 Brier 0.0614・HGB 基礎 0.0610 はいずれも純市場
  0.0551 より明確に悪い。HGB 基礎の flat ROI 76.7% は baseline を上回るが、**校正（Brier/LogLoss）は market に
  劣るまま**で、単一窓の flat ROI（高オッズ的中由来で高分散）であり信頼できるエッジではない。
- ADR 0027（精度の主レバーは市場ブレンド）・ADR 0052（純モデルは市場に劣る）と**向きが一致**し、モデルクラスを
  線形 PL→非線形 GBM へ広げても結論が再現された。

## 決定

**学習型 fundamental モデル（条件付きロジット/PL・非線形 GBM）への `raw_score` 置換は棄却**する。production は
現行の `raw_score`＋α=0.2 市場ブレンドを維持し、`EstimationConfig`（α=0.2 / m=10 / 冪較正）を変更しない。
これにより #309 が掲げた「モデルクラス変更で市場にエッジを出す」仮説は本検証の範囲で否定され、#272 Phase B/C
（学習モデルのサービング）は見送る。

## 理由

- 市場を入れた最適な学習モデルは β≈1 で**市場の再現に収束**し、fundamental の marginal 寄与が（線形でも木でも）
  ほぼゼロ。これは「市場が過去走 fundamental の情報を既に織り込む」ことの直接証拠で、ADR 0027/0052 と整合する。
- fundamental のみのモデルは市場に校正で劣り、本 PJ の選択基準（EV/ROI）でも、市場 ≧ モデルの校正である以上
  EV ゲート（モデル確率 × オッズ ≥ 1）が systematic な +EV を拾えない。flat ROI の単発の優位（HGB 基礎 76.7%）は
  校正の裏付けが無く分散と判断する。
- モデルクラス（線形 PL・非線形 GBM）という残レバーを出し切って改善しなかったため、現行構成の維持が妥当。

## 影響 / 留保

- production・既存挙動とも不変。本 ADR は「学習モデルへ置換しない」根拠を数値で固定する記録。`scripts/harness/`
  の学習・評価コード（`train_pl.py` / `train_gbm.py`）は再検証用に残すが production 経路には接続しない。
- **検証範囲の限定**: win 段のみ（PL の place/show 整合は未実装）、単一の expanding walk-forward（PL は L2、HGB は
  既定ハイパラで頑健性を確認したが網羅的スイープは未実施）、flat top-pick ROI 中心（live_ev の EV 選抜 ROI は
  未測定だが、市場 ≧ モデル校正のため +EV を拾えない見込み）。これらは「現状の特徴量・手法では市場を超えない」
  ことの否定であり、「将来いかなる学習モデルも不可能」の証明ではない。市場が見ない情報（調教・厩舎の自信・
  資金流入など、ADR 0027）を取り込む新規特徴量が得られれば再検討の余地はある。
- value シグナル（純モデルの高 ROI、ADR 0052 留保）の真偽は本 ADR では未解決のまま。市場包含の本結果は、
  fundamental 単体の overlay が高分散ノイズである可能性を支持する側の証拠になる。
- 市場特徴量・純市場・回収率に使う `win_odds` はダンプ上のオッズ（当時 race_odds スナップショット優先・
  無ければ PDF 確定単勝）で、bet 時点より後の情報を含みうる。ただしこれは**市場側を有利にする**方向で、
  「fundamental が市場を超えない＝棄却」の結論をむしろ保守的に強める（市場を過大評価しても fundamental は
  勝てない）ため、結論の妥当性は損なわれない。
- 依存追加: 学習・評価に numpy/scipy/scikit-learn を使う venv（`scripts/harness/requirements.txt`）。忠実性
  サニティ③（`faithfulness.py`）は引き続き標準ライブラリのみ。

## 再現方法

```sh
# 1) 全期間ダンプ（production 構成・as-of＝リーク無し）
./target/debug/paddock-analyze backtest --from 2025-01-05 --to 2026-06-14 \
  --shrinkage-m 10 --win-power 1.25 --place-show-power 2.0 --blend-alpha 0.2 \
  --dump-features scripts/harness/data/dump_full.tsv

# 2) venv 構築
python3 -m venv scripts/harness/.venv
scripts/harness/.venv/bin/pip install -r scripts/harness/requirements.txt

# 3) walk-forward 評価（PL / HGB）
scripts/harness/.venv/bin/python scripts/harness/train_pl.py  scripts/harness/data/dump_full.tsv
scripts/harness/.venv/bin/python scripts/harness/train_gbm.py scripts/harness/data/dump_full.tsv
# 出力の Brier/LogLoss/flat ROI で「市場あり ≒ 市場/baseline・fundamental は校正で劣る」を確認する。
```

## 関連

- Issue: #309（学習型 fundamental モデル・本検証）/ #272（予測フロー再設計・親）/ #305（純モデル value 検証, close 済み）
- ADR: 0027（精度のレバーは市場ブレンド）/ 0052（α blend 廃止＝純モデル化の棄却）/ 0042（win_power）/
  0047・0050・0051（place/show の中央圧縮＝raw_score 構造由来の校正課題）
- 設計: `docs/specifications/learned-model-harness.md`（3層＋サービングのハーネス設計、Phase A=③忠実性サニティ）
