# ADR 0012: 確率推定に調教師(trainer)統計を接続 (Issue #74)

## ステータス
承認済み

## コンテキスト
`HorseResult.trainer` は取り込まれているが確率推定で未使用。調教師の実績（厩舎の傾向）は予測に
効く変数で、既存の `jockey_stats`（騎手統計）と同じ枠組みで追加できる（#74）。

調教師名は出馬表 `HorseEntry` に無いため、入手経路が設計上の論点:
- 本番 predict 経路では **netkeiba 出馬表から trainer を抽出**して `HorseEntry.trainer` に乗せる
  （`td.Trainer` の `title` 属性、フィクスチャ裏取り済み）。
- 出馬表 PDF パーサ（entry-parser）は調教師欄の x 座標が実物サンプルなしに確定できず、本 ADR では
  未対応（別 Issue）。PDF 経路で取り込んだレースは `trainer=None`（項なし）。
- backtest 経路は `results.trainer`（当該レース確定値）を使う（predict と対称）。

## 決定

1. **`HorseFactors` に `trainer_surface: Option<RateTriple>` を追加**し、`raw_score` の重み付き
   平均に `TRAINER_WEIGHT` で組み込む。欠落（調教師なし・該当 surface 実績なし）は項と重みを母数から
   除外（`stat_to_triple_opt`、ADR 0007/0011 の流儀。「実績なし」を 0 レートと区別）。

2. **`trainer_stats` を新設**（`jockey_stats` を `results.trainer` で複製、`by_surface`/`by_gate_group`
   同型）。集計母数は `results.trainer`。`results(trainer)` にインデックスを追加。

3. **受け渡し**: predict は `entry.trainer`（netkeiba 出馬表）、backtest は `r.trainer`（results）。
   `save_race_card` の ON CONFLICT 更新を `trainer = COALESCE(excluded.trainer, horse_entries.trainer)`
   とし、PDF 経路（trainer=None）が後から netkeiba の trainer を消さないようにする。

4. **CLI に `trainer` サブコマンド追加**（`jockey` 同型）。

## 重みの決定（測定保留）

本来 jockey/track_condition と同様に backtest で重みを検証するが、**現状 `results.trainer`・
`horse_past_runs.trainer` がいずれも空**（結果 PDF パーサが trainer を抽出しておらず、netkeiba
過去走にも trainer 列が無い）。よって backtest では trainer 項が一切発火せず、重みを変えても
結果は不変（before = after, 2026-03-28〜05-31, 144 レース）:

| TRAINER_WEIGHT | 単勝 | 連対 | 複勝 | 回収率 |
|---|---|---|---|---|
| 0.0 | 13.2% | 19.4% | 33.3% | 69.1% |
| 1.0 | 13.2% | 19.4% | 33.3% | 69.1% |

母数が空のため測定不可。同種の RateTriple 項である `jockey_surface` と同じ **1.0** を採用する
（過適合リスク低・概念的に一貫）。**母数充足後に backtest で再検証すること**。

## 理由
- jockey を完全踏襲して実装でき、欠落の Option 除外も ADR 0007/0011 と一貫する。
- netkeiba 出馬表からの trainer 取得は確実（フィクスチャ裏取り済み）。PDF 経路の trainer 抽出と
  統計母数の充足は独立作業として別 Issue 化し、本 PR は配線の骨格を提供する。

## 影響
- 配線は完成しテスト通過（domain 減点なし / predict・backtest 配線 / netkeiba 抽出「田中博」/
  COALESCE 保持）。
- **ただし統計母数（`results.trainer` 等）が空のため、本機能は実データ上は現状無効**。母数充足の
  別 Issue 完了後に効果が出る。netkeiba での新規出馬表取り込みは `horse_entries.trainer` を埋めるが、
  trainer_stats の集計母数は `results.trainer` 依存のため、それだけでは統計が出ない点に注意。
- `save_race_card` の COALESCE 追加で、netkeiba→PDF の取り込み順でも trainer が保持される。
- 単調性（`win ≤ place ≤ show`, ADR 0007）は保持される。

## 関連
- ADR 0007（欠落項の母数除外）/ ADR 0011（実績なし≠全敗の区別, #73）/ ADR 0009（Optional 項追加の前例）
- 別 Issue: (a) trainer 統計の母数充足（結果 PDF / netkeiba 過去走の trainer 抽出と backfill）、
  (b) 出馬表 PDF パーサ（entry-parser）の trainer 抽出
- 設計書 `docs/specifications/probability-estimation.md`
