# 0057. 欠落 stat factor をレース内 field mean で補完し純モデルの resolution を改善（採用）

## ステータス

採用（#272 改善②で実装）。`EstimationConfig::impute_missing_factors`（`production()` で有効）＋ `estimate.rs`/`scoring.rs`。CLAUDE.md の買い方ルールは不変。

## コンテキスト

改善①（ADR 0056・PR #320）で純重み空間の果実は取り切った（本 dump 上の再スイープでも純重み最良 alt は AUC +0.0013 だが top1 −0.0026 で resolution 主指標は悪化）。次の伸び代を Phase A 診断（`feature-resolution-diagnosis.md`）で探すと、**識別力と ablation の乖離**が浮かぶ:

- `horse_surface`（欠落 0.285・corr 0.255）・`horse_distance`（0.351・0.248）・`horse_track_condition`（0.393・0.241）は **corr（識別力）が高いのに ablation で外しても top1 が変わらない〜僅かに改善**。
- 原因は現行 `raw_score` の欠落処理。欠落 factor はその馬で **項ごと母数から落とす（drop）**（ADR 0007/0014）。すると同レースで当該 factor を**持つ馬だけがシグナルを得て、欠く馬とのレース内相対比較が失われる**。欠落率 28〜39% の高欠落 factor では識別力が構造的に希釈される。

なぜ今効くか: #272 Phase B（ADR 0055）で **EV 層は純モデル**を使う。純モデルの resolution 改善は EV/decision-support の信号品質を直接上げる。物差しは **Brier/AUC/top1（ROI でない, ADR 0055）**。

## 決定

欠落 stat factor を **drop せず、同レース内 present 馬の縮約後レート平均（＝field mean）で補完**する。present が 2 頭未満のときは平均が単一馬に潰れて中立にならないため prior で埋める。scalar 項（recent_form 等）は補完対象外（従来どおり drop）。

- `scoring.rs`: `FactorImpute`（factor 別補完値）＋ `raw_score_with_impute`。`from_field` がレース単位で per-selector（win/place/show）の field mean を作る。`raw_score` は全 drop の `FactorImpute::DROP` を渡す test 用 wrapper に退避。
- `estimate.rs`: `impute_missing_factors` 有効時に per-selector の `FactorImpute` を計算して適用（無効時は DROP＝現行と厳密一致）。
- `config.rs`: `EstimationConfig::impute_missing_factors`（`Default`=false / `production()`=true）。backtest は `--impute-missing-factors` で A/B。

補完は全 6 stat factor に一律適用する（「欠落＝レース内中立」の単一ポリシー）。低欠落 factor（jockey 2.2%・trainer 1.5%）は発火頻度が低く、high-miss 3 factor 限定と screening 上ほぼ同値（top1 0.1959 vs 0.1966）で、一律の方が単純かつ僅かに上。

## 検証（measure→implement→validate）

**screening（Python ミラー・`scripts/predict-check/impute_prototype.py`・素性レート列から再計算するため dump の補完有無に非依存）**: 補完戦略 × 対象 factor を掃引。`race_mean [all stat]` が最良（下表）。`prior` 補完も改善するが `race_mean`（レース内中立）が上。忠実性は drop ≡ 現行 `race_probs`（Δ=0）で担保。

| 案（純 α=1.0, gated 4,594R） | AUC | top1 |
|---|---|---|
| baseline（drop＝現状） | 0.6708 | 0.1824 |
| **race_mean [all stat]（採用）** | **0.6781** | **0.1966** |
| race_mean [high-miss 3] | 0.6781 | 0.1959 |
| prior [all stat] | 0.6760 | 0.1946 |

**Rust 実装の二段ガード（実 backtest, 2025-01〜2026-06）**:

| 指標 | baseline(drop) | 補完(field mean) | 判定 |
|---|---|---|---|
| 純 AUC(win) | 0.671 | **0.678** | +0.007 改善 |
| 純 top1 | 0.182 | **0.197** | +0.015 改善（全6四半期改善） |
| **blended 単勝 Brier/LogLoss**(α=0.2) | 0.0544/0.1953 | 0.0544/0.1953 | 完全不変 |
| **blended 連対 Brier/LogLoss** | 0.1043/0.3553 | **0.1040/0.3521** | 改善 |
| **blended 複勝 Brier/LogLoss** | 0.1446/0.4606 | **0.1445/0.4566** | 改善 |
| blended 単勝/連対/複勝 的中 | 31.3/50.9/64.0% | 31.2/50.9/64.0% | 実質フラット |
| #258 複勝圏 1-3位 | 44.9% | **45.8%** | 取りこぼし改善 |
| place 買い目 ROI | 76.8% | **82.0%** | 改善 |

- 純 resolution は AUC/top1 とも改善・全6四半期で安定。Rust 出力（dump の `model_win`）が Python プロトタイプの `race_mean [all stat]` 予測と一致（忠実性 1.1e-16, `impute_prototype.py --verify-dump`）。
- **production blended（α=0.2）は非回帰**: 単勝校正は完全不変、連対/複勝校正・複勝 ROI・#258 取りこぼしは改善。単勝的中/想定回収の −0.1pt は丸め誤差、quinella 買い目 ROI −2.6pt は curated 買い目（点数 38→40）のサンプルノイズで、連対校正はむしろ改善している。

## 理由

- 高欠落 factor の識別力は corr で確認できるのに、drop 処理がレース内で「持つ馬 vs 欠く馬」の比較を壊して活かせていなかった。field mean は present 馬の相対差を保ったまま欠く馬を中立に置くため、識別力を希釈せず引き出せる。
- 「実績なし ≠ 全敗（0 レート）」の方針（ADR 0007/0014）は維持。field mean（present<2 は prior）はレース内中立＝減点でない。drop より原理的に妥当な欠落処理への更新。
- 物差しは **確率の正しさ（Brier/AUC/top1）であって ROI でない**（ADR 0055）。

## スコープ外

- レース内 z-score/rank 等の素性再設計（ADR 0056 で測定し悪化＝不採用）。scalar 項の補完（今回は stat のみ）。
- isotonic（診断で棄却）・配分/Kelly/買い方ルール・blend α・学習モデル（ADR 0053 棄却）。

## 影響

- `config.rs`/`scoring.rs`/`estimate.rs` と CLI（`--impute-missing-factors`）。`production()` 既定で predict/EV 層に反映。`Default` は false のため既存の default-config 呼び出し・テストは挙動不変。
- 測定ツール `scripts/predict-check/impute_prototype.py`（掃引＋ `--verify-dump` 忠実性）。診断ツール `feature_resolution_diag.py`/`weight_sweep.py` は BEFORE 分解を記録する #319/#320 の成果物として不変（drop 母数の分解を保つ）。
- `docs/specifications/probability-estimation.md` の欠落処理を更新。
- 関連: 0055（EV 層分離・純モデル化）/0056（改善①重み再調整）/0027（精度の主レバー＝市場ブレンド）/0007・0014（欠落項の母数除外方針）/0053（学習モデル棄却）。

## 再現

```sh
# 純モデル dump（補完ありでビルド後）＋忠実性・resolution
./target/release/paddock-analyze backtest --from 2025-01-01 --to 2026-06-30 \
  --blend-alpha 1.0 --shrinkage-m 10 --win-power 1.25 --place-show-power 2.0 \
  --impute-missing-factors --dump-features /tmp/pa/pure_impute.tsv
python3 scripts/predict-check/impute_prototype.py --tsv /tmp/pa/pure_impute.tsv --verify-dump  # 1.1e-16
# 掃引（drop vs 補完戦略の before/after は素性レート列から再計算するためどの純 dump でも可）
python3 scripts/predict-check/impute_prototype.py --tsv /tmp/pa/pure_impute.tsv
# blended 非回帰（--impute-missing-factors の on/off で A/B）
./target/release/paddock-analyze backtest --from 2025-01-01 --to 2026-06-30 \
  --blend-alpha 0.2 --shrinkage-m 10 --win-power 1.25 --place-show-power 2.0 --impute-missing-factors
```
