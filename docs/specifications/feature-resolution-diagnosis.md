# 純モデル確率の素性分解診断（#272 Phase A）— resolution か calibration か

## 結論（先に）

**純モデルは resolution 限定（本命を見分けるランク自体が弱い）。isotonic 較正は効かない。次は素性/モデル改善に進む。**

- isotonic（calibration 較正）は市場との Brier gap を **わずか 1.0% しか詰めない** → calibration の問題ではない。
- 純モデルの本命当て（top1）・順位相関・AUC が市場に **大きく・全窓で安定的に劣る** → ランク（resolution）が弱い。
- ランクが弱い以上、単調変換（isotonic）では届かない。**素性の使い方を直すのが筋**。

## 方法（measure→prescribe・production コード変更なし）

- 入力: `paddock-analyze backtest --from 2025-01-01 --to 2026-06-30 --blend-alpha 1.0 --shrinkage-m 10 --win-power 1.25 --place-show-power 2.0 --dump-features /tmp/pa/pure.tsv`（as-of・リーク無し。68,148 行 / 4,891 レース）。
- 解析: `scripts/predict-check/feature_resolution_diag.py`（標準ライブラリのみ）。Rust の確率推定パイプライン（raw_score→shrinkage→score_power→normalize→win_power）を Python で鏡映。
- **忠実性アンカー: `max|python_win − dump model_win| = 1.7e-16`** → 鏡映は厳密一致。以降の数値はすべて有効。

## 計測結果

### (1) resolution（純モデル vs 市場・全期間 n=4,594 レース）

> n=4,594 は全 4,891 レースのうち **勝馬（1着）が記録され かつ オッズがある（≧1頭）レースのみ**（`any(y_win) and s>0` で 297 レースを除外）。純モデルと市場を同一レース集合で比較する（top1 の分母も両者同一の n=4,594）。

| 指標 | 純モデル | 市場 |
|---|---|---|
| top1 的中率（その馬が1着） | **0.162** | 0.333 |
| Spearman（レース内 確率 vs 着順） | 0.223 | 0.534 |
| AUC（win, 全馬） | **0.649** | 0.833 |
| Brier（win） | 0.0659 | 0.0574 |
| LogLoss（win） | 0.2566 | 0.2318 |

純モデルは winner を最上位に置けるのが市場の半分（16% vs 33%）。AUC 0.649 は弱い（0.5=ランダム）。

### (1b) 窓別安定性（四半期・全窓で同じ向き）

| 四半期 | races | top1_model | top1_market | AUC_model | AUC_market |
|---|---|---|---|---|---|
| 2025Q1 | 744 | 0.129 | 0.341 | 0.611 | 0.840 |
| 2025Q2 | 761 | 0.168 | 0.344 | 0.644 | 0.838 |
| 2025Q3 | 844 | 0.159 | 0.333 | 0.643 | 0.827 |
| 2025Q4 | 773 | 0.184 | 0.323 | 0.666 | 0.822 |
| 2026Q1 | 806 | 0.200 | 0.339 | 0.673 | 0.834 |
| 2026Q2 | 666 | 0.126 | 0.317 | 0.657 | 0.836 |

純モデル AUC は毎窓 0.61–0.67、市場は 0.82–0.84。gap ~0.17–0.22 が安定。単一窓ノイズではない。

### (2) 素性別 識別力（欠落率・レース内分散・複勝率との相関）

| factor | 重み | 欠落率 | レース内分散(平均) | corr(show率, 複勝) |
|---|---|---|---|---|
| **course_gate** | **2.0** | 0.042 | **0.00125（最小）** | **0.031（≒無相関）** |
| horse_surface | 1.0 | 0.285 | 0.00381 | 0.255 |
| horse_distance | 1.0 | 0.351 | 0.00347 | 0.248 |
| **jockey_surface** | 1.0 | 0.022 | **0.00802（最大）** | 0.243 |
| trainer_surface | 1.0 | 0.015 | 0.00447 | 0.147 |
| horse_track_condition | 1.0 | 0.393 | 0.00299 | 0.241 |
| recent_form | 0.25 | 0.191 | (scalar) | — |
| weight_carried | 0.25 | 0.082 | (scalar) | — |
| jockey_recent_form | 0.0 | 0.016 | (無効) | — |

**最大重み 2.0 の `course_gate` が最も識別力が無い**（レース内でほぼ一定・複勝とほぼ無相関）。場×枠のベース率で、同一レースの全馬がほぼ同値＝順位を作らないのに、最大の重みで他の識別素性を希釈している。識別力は `jockey_surface`／`horse_surface`／`horse_distance`／`track_condition`（corr 0.24–0.26）にあるが、後3者は欠落率が高い（0.28–0.39）。

### (3) leave-one-out ablation（外して悪化＝有用／改善＝害。Δ は baseline 比）

| 外した factor | ΔBrier | ΔLogLoss | Δtop1 |
|---|---|---|---|
| jockey_surface | +0.0005 | +0.0036 | **−0.040** |
| trainer_surface | +0.0002 | +0.0020 | −0.009 |
| course_gate | −0.0000 | +0.0015 | −0.011 |
| weight_carried | −0.0000 | +0.0033 | +0.006 |
| recent_form | −0.0000 | +0.0012 | −0.002 |
| horse_surface | −0.0000 | −0.0002 | **+0.005** |
| horse_distance | −0.0001 | −0.0004 | **+0.005** |
| horse_track_condition | −0.0000 | −0.0001 | **+0.007** |

`jockey_surface` 除去が最も悪化＝本モデルの主シグナル。`trainer_surface` も有用。一方 `horse_surface/distance/track_condition` は除去で top1 が**改善**＝現状の重み・欠落の扱いでは弱い/僅かに害。ただし Δ の絶対値は小さい（モデルが全体にフラットなため一素性の振れも小さい）。**最適重みは別途 sweep で測る**（本 ablation は現行重みでの寄与であり最適化ではない）。

### (4) 正規化の圧縮度

`mean var(raw_score_win)=0.00068`、`mean var(model_win)=0.00042`、圧縮比 0.62。正規化で分散が ~4 割落ちるが、そもそも **raw_score の分散が極小**（縮約後のレートを重み付き平均する構造上、レース内で値が割れない）。フラット化は「素の分散が小さい × 正規化で更に潰れる」の複合。

### (5) isotonic 上限効果（walk-forward 6 窓・前窓 fit→後窓適用）

`Brier(win)` pure 0.0662 → **pure+isotonic 0.0661**、market 0.0579。**isotonic は市場との gap を 1.0% しか詰めない。** ランクが弱い対象に単調較正をかけても resolution は生まれない。

## 判定と次ラウンドの方針（別 PR・本診断で go）

**resolution 限定が確定。isotonic 実装は棄却（効果 1.0%）。** 次は **素性/モデルの resolution 改善**:

1. **`course_gate` の重み 2.0 を見直す**（最有力の改善点）。最大重みなのに識別力ゼロで希釈源。weight sweep（0/0.5/1.0/2.0）と、場×枠を「レース内で差が出る形」に作り直せるか（例: 当該馬の枠 vs フィールド相対）を検討。※過去 #87/ADR 0012 で course_gate=2.0 を採用済みのため、当時と同じ backtest 物差し＋本診断の resolution 指標で再評価する。
2. **識別素性（jockey/trainer/horse_surface/distance）の活かし方**: 欠落率の高い horse_surface/distance（0.28–0.35）の欠落補完、jockey/trainer の重み再配分。
3. **raw_score の分散不足そのもの**: 重み付き平均（→中心化）でなく、レース内 z-score 化やランク特徴など「レース内で割れる」素性設計を検討。
4. **物差しは calibration/resolution（Brier・AUC・top1・reliability）であって ROI でない**（ADR 0055）。各案は backtest で resolution が上がるかを測ってから採用。

公開データの天井は市場≈（ADR 0027）だが、現状 AUC 0.649 は市場 0.833 に**大きく届いておらず、公開データの天井よりかなり下**＝素性改善の伸び代は大きい（市場再現が目的ではなく、純モデルの確率を素直に良くする）。**※この「伸び代は大きい」は後日の arc で否定された。下記「到達点」を参照。**

### 到達点（2026-07-02・arc 完了後の追記）

上の「伸び代は大きい」は arc を回した結果**否定された**。改善①（重み再調整・ADR 0056）＋改善②（欠落 factor の field mean 補完・ADR 0057）で純 AUC 0.649→0.678・top1 0.162→0.197 まで改善し merged。その後**既存データの resolution レバーは全滅**（within-race 相対化＝ADR 0056・recency＝ADR 0034・クラス昇降＝class_prototype 撤退）、**新データ（血統/種牡馬）も measure-first ゲートでノイズ級・棄却（ADR 0058）**。純 AUC 0.678 vs 市場 0.833 の残り gap は**素性追加でも coverage 拡大でも詰まらない**ことが確定。

**天井は coverage でなく factor 冗長性**（ADR 0058 訂正で確定）。gated 4,594R を「1レース内の馬履歴 factor（horse_surface/distance/track_condition）カバー率」で層別すると、model AUC はフラット（0.65-0.685）で、フル装備の 100% 層(0.685) は履歴ゼロの 0% 層(0.677) を +0.008 しか上回らない＝馬履歴 factor は常在の course_gate/jockey/trainer に冗長（`/tmp/pa/coverage_strata.py`）。※ ADR 0058 初版の「coverage cap 19.5%」は sire を `results.horse_id`(20.6%) で join したアーティファクトで馬 factor 一般の天井ではない（実 coverage ~60-71%）＝同 ADR 訂正節参照。全 runner 履歴の大量 fetch arc は不要。次に動かすなら公開データ外の情報が要る（ADR 0027）。層別ツール `scripts/predict-check/coverage_strata.py`。

## 再現

```sh
# ダンプ生成（DB 読み込み・重い／共有 DB 競合に注意）
./target/release/paddock-analyze backtest --from 2025-01-01 --to 2026-06-30 \
  --blend-alpha 1.0 --shrinkage-m 10 --win-power 1.25 --place-show-power 2.0 \
  --dump-features /tmp/pa/pure.tsv
# 診断（標準ライブラリのみ）
python3 scripts/predict-check/feature_resolution_diag.py --tsv /tmp/pa/pure.tsv
# 鏡映関数の単体テスト
python3 scripts/predict-check/test_feature_resolution_diag.py
```
