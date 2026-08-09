# 0056. 素性重み再調整（course_gate 2.0→1.0・jockey_surface 1.0→2.0）で純モデルの resolution を改善（採用）

## ステータス

採用（#272 改善①で実装）。`weights.rs` の定数変更。CLAUDE.md の買い方ルールは不変。

## コンテキスト

Phase A 診断（ADR 0055 の follow-up・PR #319・`docs/specifications/feature-resolution-diagnosis.md`）で純モデルは **resolution 限定**（本命を見分けるランクが弱い。top1 0.162/市場0.333・AUC 0.649/市場0.833・全6四半期で安定）と確定。isotonic は市場との Brier gap を 1.0% しか詰めず棄却。診断の素性別所見:

- **最大重み 2.0 の `course_gate` が最も識別力ゼロ**（レース内分散最小・複勝相関0.031≒無相関）。場×枠のベース率で同一レースの全馬がほぼ同値＝順位を作らないのに、最大重みで識別素性を希釈していた。
- **主シグナルは `jockey_surface`**（leave-one-out で top1 を最も落とす素性, −0.040）・`trainer_surface`。

なぜ今これが効くか: #272 Phase B（ADR 0055・PR #318）で **EV 層は純モデルを使う**。純モデルの resolution 改善は EV/decision-support の信号品質を直接上げる。

過去の重みは小窓（144R）の blended Brier 中心で決めた（course_gate=2.0 は #87 系）。本件は **4,594R の純モデル resolution** という別物差し・高標本での再評価である。

## 決定

`src/domain/src/prediction/weights.rs` の重みを変更:
- **`COURSE_GATE_WEIGHT` 2.0 → 1.0**（識別力ゼロの希釈源を是正。0 まで下げると top1 が落ちるため 1.0 を採る）。
- **`JOCKEY_WEIGHT` 1.0 → 2.0**（純モデルの主シグナルを増強）。

他の factor 重み・shrinkage・冪変換・blend α は不変。

## 検証（measure→implement→validate）

**スイープ（Python ミラー・既存 dump 上・忠実性 1.7e-16 実証。1.1e-16 は実装後の新 dump 再確認値）**: `scripts/predict-check/weight_sweep.py`。`course_gate` を下げ `jockey_surface` を上げると純 AUC/top1 が単調改善。`course_gate=1.0, jockey_surface=2.0` が頑健（全6四半期で AUC/top1 改善）。`within-race z-score` prototype は悪化（採用せず）。

**Rust 実装の二段ガード（実 backtest, 2025-01〜2026-06）**:

| 指標 | baseline(旧重み) | 新重み | 判定 |
|---|---|---|---|
| 純 AUC(win) | 0.649 | **0.671** | +0.022 改善 |
| 純 top1 | 0.162 | **0.182** | +0.020 改善 |
| 純 Brier(単) | 0.0659 | 0.0655 | 改善 |
| **blended 単勝的中**(α=0.2) | 31.2% | 31.3% | 非回帰 |
| **blended 単勝 Brier** | 0.0543 | 0.0544 | flat（誤差） |
| **blended 複勝 Brier** | 0.1461 | **0.1446** | 改善 |
| **blended 連対/複勝 LogLoss** | 0.3576/0.4644 | **0.3553/0.4606** | 改善 |

- 純モデル resolution は AUC/top1 とも有意改善・全6四半期で安定。Rust 出力（dump の `model_win`）が Python スイープ予測と一致。
- **production blended（α=0.2）は非回帰**: 単勝は flat、連対/複勝の校正はむしろ改善。本番出荷モデルを悪化させない。

## 理由

- course_gate を最大重みに据える根拠は **小窓 blended Brier**（#87）だった。blended は市場（α=0.8）が支配的で純モデル素性の識別力が見えにくく、course_gate の希釈が顕在化しなかった。4,594R の純モデル resolution で測ると害が定量化される。
- jockey_surface は ablation で純モデルの最重要シグナルと出ており、増強が resolution を直接押し上げる。
- 物差しは **Brier/AUC/top1（確率の正しさ）であって ROI でない**（ADR 0055）。儲けでなく確率の正しさを改善した。

## スコープ外

- レース内 z-score/rank 等の素性再設計（Python で測定し悪化＝不採用。raw_score 構造変更はしない）。
- isotonic（診断で棄却）・配分/Kelly/買い方ルール・blend α・学習モデル（ADR 0053 棄却）。

## 影響

- `weights.rs` の 2 定数変更。predict/backtest/EV 層すべてに純モデル重みとして反映。
- Python ミラー（`feature_resolution_diag.py` の `STAT_FACTORS`）も production 重みに同期（忠実性 1.1e-16 で再確認）。
- `docs/specifications/probability-estimation.md` の重み式を更新。
- 関連: 0055（EV 層分離・純モデル化）/0027（精度の主レバー＝市場ブレンド）/0042（win-power）/0047（place/show 脱圧縮）/0012・#87（旧重みの根拠）。

## 再現

```sh
# 純モデル dump（新重みでビルド後）
./target/release/paddock-analyze backtest --from 2025-01-01 --to 2026-06-30 \
  --blend-alpha 1.0 --shrinkage-m 10 --win-power 1.25 --place-show-power 2.0 \
  --dump-features /tmp/pa/pure_new.tsv
python3 scripts/predict-check/feature_resolution_diag.py --tsv /tmp/pa/pure_new.tsv   # AUC/top1 と忠実性
python3 scripts/predict-check/weight_sweep.py --tsv /tmp/pa/pure_new.tsv              # 重みスイープ（素性レート列から再計算するため dump の重みに非依存・どの dump でも可）
# マージ後は weights=None の baseline が新重みを指すため、before(0.649/0.162) は candidates の
# "old (cg=2.0 jk=1.0)" 行、after(0.671/0.182) は "cg=1.0 jk=2.0 (採用)" 行で対比できる。
# blended 非回帰（新旧重みの binary で）
./target/release/paddock-analyze backtest --from 2025-01-01 --to 2026-06-30 \
  --blend-alpha 0.2 --shrinkage-m 10 --win-power 1.25 --place-show-power 2.0
```
