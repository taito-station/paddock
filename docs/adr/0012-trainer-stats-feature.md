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

## 重みの決定（#87 で母数充足・backtest 再検証済み）

### 経緯
配線当初（#74）は **`results.trainer`・`horse_past_runs.trainer` がいずれも空**で backtest の trainer 項が
一切発火せず、重みを変えても結果は不変（before = after）だった。#82 で結果 PDF からの trainer 抽出を
stext 座標方式で実装し `results.trainer` を充足できるようにしたが、再取込できたのが手元 8 レースのみで
母数が薄く、暫定 1.0 据え置きとした。

### #87 での再検証
JRA から結果データを再取得して測定 DB に **trainer 母数を充足**（測定 DB 全体で 476 レース・trainer
充足 99%）し、そのうち backtest window 該当の 144 レースで `TRAINER_WEIGHT` をスイープ再検証した
（**#81 後の None 母数除外ロジック**上、2026-03-28〜05-31）。回収率・的中率は参考値で重み選定の
根拠にはせず、校正指標（Brier / LogLoss・小さいほど良い）で判断する:

| TRAINER_WEIGHT | 単勝 | 連対 | 複勝 | 回収率※ | Brier(単) | Brier(連) | Brier(複) | LogLoss(単) |
|---|---|---|---|---|---|---|---|---|
| 0.0 | 10.4% | 16.7% | 28.5% | 36.1% | 0.0657 | 0.1254 | 0.1683 | 0.6006 |
| 0.5 | 11.1% | 17.4% | 28.5% | 39.5% | 0.0638 | 0.1196 | 0.1604 | 0.4013 |
| **1.0** | 9.7% | 15.3% | 27.8% | 38.9% | 0.0635 | **0.1189** | **0.1595** | **0.3998** |
| 2.0 | 10.4% | 16.7% | 31.2% | 43.3% | 0.0634 | **0.1189** | 0.1598 | 0.4004 |

※ 回収率・的中率は「トップ選好馬の単勝に毎レース 100 円」固定の参考値で、144 レースのノイズが大きく
重み選定の根拠外。

### 結論
- **trainer 項を有効化する（重みを 0 より大きくする）と、重みの大小に依らず校正が明確に改善**する
  （0.0 → 0.5 で LogLoss 単勝 0.60→0.40、Brier も低下）＝調教師シグナルは有効。母数充足により
  before=after が解消し、項が実際に発火することを確認した。
- 有効化したうえで 0.5 / 1.0 / 2.0 は校正がほぼ拮抗。その中で **1.0 が LogLoss 単勝・Brier 複勝で最良**
  （Brier 連対は 1.0 と 2.0 が 0.1189 で同率最良）。Brier 単勝のみ 2.0 が僅差で良いが過適合を避け 1.0 を採る。
- よって **TRAINER_WEIGHT = 1.0 を確定**（暫定ではなく実測検証済み）。同種 RateTriple 項 `jockey_surface`
  と同値で概念的にも一貫。

### 補足（母数の再現性）
測定は結果データ再取得で構築した測定 DB（results のみ・finishing_position は OCR 充足）で実施。実運用
DB（`data/paddock.db`）の母数充足は本検証には不要なため未実施（任意の運用フォローアップ）。なお live
predict は entry.trainer（netkeiba 略名）で join するため、live 経路で trainer 項を発火させるには略名↔
フルネームの正規化が別途必要（既知課題）。

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
- `trainer_surface` は実績なしを `None`（母数除外）とするが、既存の `jockey_surface` は旧仕様の
  0 埋め（実績なし=0レートで減点側）を踏襲しており、同じ `Option<RateTriple>` 項ながら欠落扱いが
  非対称。jockey 等の 0 埋めを `None` 除外へ統一するかは #81 で別途検討する。

## 関連
- ADR 0007（欠落項の母数除外）/ ADR 0011（実績なし≠全敗の区別, #73）/ ADR 0009（Optional 項追加の前例）
- 別 Issue: (a) trainer 統計の母数充足（結果 PDF / netkeiba 過去走の trainer 抽出と backfill）、
  (b) 出馬表 PDF パーサ（entry-parser）の trainer 抽出
- 設計書 `docs/specifications/probability-estimation.md`
