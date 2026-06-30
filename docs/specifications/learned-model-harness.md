# 学習型モデル評価ハーネス 設計（#272 土台 / #309 受け皿）

手作り線形 `raw_score` を学習ランカー（#309）へ置換する前提として、**リーク無しで訓練・評価でき、
任意のモデルを production の EV/買い方ロジックで対市場評価できる共通基盤**の設計。本書は #272 の土台
（分析と市場の分離・walk-forward 計測）と #309（学習モデル実装）が共有する。

## 背景：なぜ必要か（value シグナルの実証）

リーク無し `analyze backtest`・production 構成（m=10 / win_power=1.25 / place_show_power=2.0）・4891R で
α∈{0.0, 0.2, 1.0} を比較した（#272 コメント）:

| α | 単勝的中 | フラット回収率 | EV選抜 win 買い目 |
|---|---|---|---|
| 0.0 純市場 | 31.3% | 75.6% | 2点・ROI≈0%（+EV の win がほぼ無い） |
| 0.2 現行 | 31.2% | 75.4% | 1点・ROI≈0% |
| 1.0 純モデル | 15.2% | 80.1% | 251点・**ROI 98.2%** |

- **純モデルだけが +EV の単勝を多数（251点）見つけ ROI 98.2%**。市場・現行は efficient で食い違いを作れない。
- ただし **98.2% < 100%（赤字）**。ADR 0052 はこの value シグナルを「**未否定（要検証）**」に留め、(1) 点推定のみで
  分散未計測、(2) 純モデル回収率は母数 852 の非ランダム部分集合で選択効果が乗りうる、(3) blend を外すと精度崩壊、の
  3 留保を置いた（真偽は未決）。この value 検証は **#305 で提起**（同 issue はクローズ済み）され、その切り分けを
  本ハーネス（#272/#309）が引き取る。本ハーネスは複数窓・out-of-sample でこれを検証する基盤でもある。
- → 仮にエッジが本物なら、レバーはモデルの識別力（#309）。98.2% を 100% 超へ押すには本ハーネスが要る。

ADR 0052（α blend 廃止＝純モデル化の棄却）の通り、純 P_model を EV に直接使う素朴案は不可（校正が崩れる）。
本ハーネスは「強い学習モデルを安全に訓練・評価し、勝てたら採用する」ための仕組みであり、設計を変えずに
モデルを差し替えられる継ぎ目を提供する。

## アーキテクチャ（3層 + サービング）

```
① 特徴量エクスポート(Rust)  →  ② オフライン訓練(Python, walk-forward)  →  ③ 評価(Python, 対市場)
                                                                              ↓ baseline 超えなら
                                                                          ④ サービング(Rust)
```

### ① 特徴量エクスポート（Rust：`analyze backtest --dump-features <PATH>`）

既存 `analyze backtest` の per-race ループ（`src/use-case/src/interactor/race/backtest.rs`：`entry_factors`
構築〜`HorseOutcome` 突合）に**ダンプ経路を追加**する。backtest は既に**as_of（`races.date < D`）でリーク無しに
全特徴量を日次バッチ取得**しているため、その値をそのまま emit すれば production 特徴量に忠実 かつ 未来リーク無し。

1 行 = 1 レース×1 馬。列（`HorseFactors` 9 項＋ラベル＋市場）:

各 `FactorStat`（6 レート項）は `rate: RateTriple{win, place, show}` と `starts: u32` を持つため、
**1 項につき win/place/show の 3 レート＋ starts の 4 列**を出す（縮約・信頼度を学習側で扱えるように）。

| 群 | 列 |
|---|---|
| キー | `race_id, date, horse_num` |
| 特徴(レート×6項) | 各項 `{factor}_win_rate, {factor}_place_rate, {factor}_show_rate, {factor}_starts`。`{factor}` ∈ `course_gate, horse_surface, horse_distance, jockey_surface, trainer_surface, horse_track_condition`（計 24 列） |
| 特徴(シグナル) | `recent_form, weight_carried, jockey_recent_form`（各 [0,1]、欠落は空） |
| ラベル | `finishing_position`（→ win=1着 / place=2着内 / show=3着内 を下流で導出） |
| 市場 | `win_odds, popularity`（backtest の確定オッズ突合と同じく post 時点既知＝リーク無し。ADR 0027 影響節「`results.odds` は post 時点で既知＝リークなし」と同基準） |

- 欠落項（`Option=None`）は**空セルのまま**出す（木はネイティブ対応、logit は欠損指標で対応）。0 埋めしない。
- count 列名は `starts`（ドメインのフィールド名 `FactorStat.starts` に合わせる）。
- 出力 TSV（将来 Parquet 可）。`--dump-features` 未指定なら既存挙動・出力は完全に不変。

実装方針（clean-arch 準拠）: interactor は file IO せず、ダンプ要求時のみ per-horse 行を `BacktestReport`
の新規 optional フィールドに収集し、`src/apps/analyze/src/bin.rs` が TSV を書く。未要求時は収集自体を行わない。

### ② オフライン訓練（Python：walk-forward）

- **日付で分割**：`date < cutoff` で訓練 → 前方窓 `[cutoff, cutoff+Δ)` を予測。cutoff をローリング
  （expanding or sliding）し、全期間の **out-of-sample 予測**を得る（構造的にリーク無し）。
- モデル（まず1手法で小さく）：
  - 条件付きロジット / Plackett-Luce（レース内 softmax。競馬＝多項選択の王道、win/place/show 整合）。または
  - LightGBM ranker（非線形・交互作用、中央圧縮を直接緩和）。
- 出力：レース×馬の out-of-sample win（必要なら place/show）確率 TSV。

### ③ 評価（Python：対市場ハーネス）

- 予測 × ラベル × 市場オッズを結合し、**production の買い方**で評価:
  - 校正：Brier / LogLoss / reliability。
  - フラット ROI（トップ選好の単勝）。
  - **EV 選抜 ROI**：`live_ev.py` のロジック（PL→exotic 確率、EV=P×odds、ROI≥100% ゲート、3券種配分）を再利用。
- 比較対象：**現行 α=0.2 baseline と純市場**。**複数窓**で curated ROI のノイズを確認（ADR 0051 の留保）。

### ④ サービング（Rust・#309 採用時）

- out-of-sample で baseline を上回ったら、重み（logit）or 木（GBM, ONNX 等）を export し、Rust predictor で
  `raw_score` と**並置**（config ゲートで切替・段階導入）。採否は ADR。

## 最重要原則：忠実性をサニティで担保

本セッションで `--shrinkage-m` の付け忘れ・zsh 単語分割で計測を誤り ADR を1本破棄した教訓から、**ハーネスの
忠実性を仕組みで保証**する:

- 特徴量は**必ず backtest と同じ as_of 経路**で emit（別計算で再現しない）。
- ③ の Python 評価は、**まず内蔵モデルの予測を①の出力から再評価し、`analyze backtest` の数値と一致することを
  サニティ**してから学習モデルに使う（ハーネス自体のバグ・設定差を検出する回帰）。
- production 構成は常に明示（m=10 / win_power=1.25 / place_show_power=2.0 / α=0.2）。

## 段階（Phase）

| Phase | 内容 | issue |
|---|---|---|
| **A** | ① 特徴量エクスポート（`--dump-features`）＋ ③ の薄い骨組み（内蔵モデル再評価で backtest 一致サニティ） | #272 |
| **B** | ② 訓練＋ walk-forward 評価 vs baseline（条件付きロジット先行） | #309 |
| **C** | baseline 超えなら ④ サービング（ADR で採否） | #309 |

## リスク / 留保

- **パリミュチュエル控除率 20-25% を net で抜くのは本質的に難しい**。98.2% を 100% 超へは数 pt だが保証はない。
- **最大リスクは overfit / リーク**。walk-forward の as_of 厳守、train/valid 分割、複数窓での再現確認が必須。
- curated ROI は単一窓・中央値近似の参考値（ADR 0051）。絶対値でなく baseline 比・複数窓で判断する。
- エンジニアリング：Python 学習 ↔ Rust 推論の境界（モデル export 形式）は Phase B/C で確定する。

## 関連

- Issue: #272（予測フロー再設計・親）/ #309（学習モデル実装）/ #305（純モデル value シグナル検証の提起元・クローズ済、検証は本ハーネス #272/#309 へ継承）/ #263（較正後 EV ゲートの逆予測性）
- ADR: 0027（精度のレバーは市場ブレンド）/ 0042（win_power）/ 0047（place/show 冪変換の採用＝`place_show_power=2.0` の根拠）/ 0050（place/show raw_score 再調整の棄却）/ 0051（place/show 冪 γ の knee 確定）/ 0052（α blend 廃止の棄却）
- 既存: `scripts/predict-check/live_ev.py`（EV/買い方ロジック）/ `docs/specifications/backtest.md` / `probability-estimation.md`
