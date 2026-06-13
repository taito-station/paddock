# ADR 0016: 少データ馬のベイズ縮約と直近成績のリーセンシー重み付け (Issue #75)

## ステータス
承認済み

## コンテキスト
確率推定（`src/domain/src/prediction/mod.rs`）は horse/jockey/trainer/course の各 factor を
**全期間一律集計のレート**で重み付け平均している。これが 2 つの弱点を生む:

1. **少データ馬の極端化**: 新馬・復帰馬は当該 factor の実績が乏しく、レートが極端になりやすい。
   実績ゼロの factor は母数除外（ADR 0014/#81）されるため、全 factor が薄い馬は `raw_score=0`→
   `win_prob=0` で実質除外される（ADR 0002 の既知制約）。
2. **直近の好不調の希薄化**: 古い成績と直近成績が同じ重みで平均され、最近の調子が反映されにくい。

ベイズ縮約（shrinkage）と直近重視（recency）でこれらを緩和できないかを検証する。パラメータ
（擬似カウント m・半減期）は backtest（ADR 0006 / #52 の Brier・LogLoss・的中率）で決め、
walk-forward の `as_of` リーク防止を厳守する。

## 決定

### 共通基盤: 切替可能な `EstimationConfig`
domain に `EstimationConfig { shrinkage: Option<..>, recency: Option<..> }` を導入し、
`estimate_probabilities_with_config` で挙動を切り替える。`estimate_probabilities`（既存）は
`Default`（両 None＝現行挙動）へ委譲し挙動不変。`analyze backtest` に `--shrinkage-m` /
`--recency-half-life` を追加し before/after を比較可能にした。

### Phase A: ベイズ縮約（採用, m=10）
各 factor のレートを母集団 prior へ `smoothed = (k·rate + m·prior)/(k + m)`（k=出走数, m=擬似
カウント）で縮約する。prior は出走頭数 ~14 由来の解析的な基準率（win=1/14, place=2/14,
show=3/14）でクエリ不要・リークなし。`HorseFactors` の 6 group factor を `Option<RateTriple>`→
`Option<FactorStat>`（レート + 出走数）へ拡張し、`raw_score` が縮約を適用する。

backtest（2026-03-28〜05-31 / 144R, ADR 0014 後ロジック）で m∈{off,5,10,20,50} を比較:

| m | 単勝的中 | 単勝Brier | 単勝LogLoss | 連対LogLoss | 複勝Brier |
|---|---|---|---|---|---|
| off | 9.7% | 0.0665 | 0.2718 | 0.4351 | 0.1626 |
| 5 | 12.5% | 0.0650 | 0.2509 | 0.3963 | **0.1601** |
| **10** | 13.2% | **0.0649** | **0.2506** | **0.3963** | 0.1605 |
| 20 | 13.9% | 0.0649 | 0.2508 | 0.3975 | 0.1612 |
| 50 | 11.8% | 0.0651 | 0.2516 | 0.3995 | 0.1623 |

m=10 が単勝 Brier・LogLoss・連対で最良、複勝も近接で、単勝的中も 9.7→13.2% と改善。m=50 は
過縮約で劣化。m=20 は的中・回収率が僅かに高いが校正は m=10 が上で、小標本での過適合を避け
**m=10 を採用**し、predict 本番のデフォルト（`EstimationConfig::production()`）に反映した。

### Phase B: リーセンシー重み付け（実装・評価のみ、デフォルト無効）
馬の芝ダ・距離帯・馬場状態 factor を、日付付き成績系列に時間減衰 `w = 0.5^(days_ago/half_life)`
を掛けた重み付きレートで評価できるようにした（domain `apply_recency_weight`、gateway の
`races.date` 別集計 `horse_recency`）。

backtest（m=10 固定 / 同 144R）で half-life∈{off,30,60,90,180} を比較:

| half-life | 単勝的中 | 単勝Brier | 単勝LogLoss | 複勝Brier |
|---|---|---|---|---|
| **off** | **13.2%** | 0.0649 | **0.2506** | **0.1605** |
| 30 | 12.5% | 0.0649 | 0.2507 | 0.1606 |
| 60/90 | 12.5% | 0.0649 | 0.2507 | 0.1606 |
| 180 | 13.2% | 0.0649 | 0.2506 | 0.1605 |

recency は校正・的中とも改善せず（4 桁目の変動）、短半減期はむしろ的中を下げた。

併用時の `apply_recency_weight` は縮約の信頼度 `k` に減衰前の素の出走数を使う非対称があるため、
この非対称が recency の効果を相殺していないかを確認すべく **shrinkage off の単独 recency** でも
スイープした（baseline=no-shrink/no-recency: 単勝 Brier 0.0665 / LogLoss 0.2718）:

| 設定 | 単勝Brier | 単勝LogLoss | 複勝Brier |
|---|---|---|---|
| 単独 baseline | 0.0665 | 0.2718 | 0.1626 |
| recency h=30 | 0.0666 | 0.2725 | 0.1631 |
| recency h=60 | 0.0665 | 0.2722 | 0.1630 |

単独でも改善せずむしろ僅かに悪化し、無改善は縮約の非対称の人工物ではないと確認できた。よって
**本番では recency を無効のまま**とし（`config.recency=None`、horse_recency も取得しないため
predict の追加コストはゼロ）、機構と CLI フラグのみ残す。

## 理由
- **縮約**: 「実績なし＝母数除外」（ADR 0014）の次の課題＝「実績が薄い factor の過信」を、原理的
  （ベイズ平滑化）に補正する。少データ馬を prior 方向へ持ち上げ `win_prob=0`（ADR 0002）を緩和
  しつつ、十分なデータの馬は生レートを保つ。backtest が校正・的中の一貫改善を示し裏付けた。
- **recency 無効**: 前走フォーム（#31, ADR 0009）が既に直近の調子を捕捉しており、馬のカテゴリ別
  出走数が疎なため時間減衰がノイズ化して改善が出ない、と解釈できる。原理的に有望でも実測で
  効果が無いものはデフォルト化しない（[[measurement-ordering]]＝挙動変更を計測で決める方針）。
  より密な jockey/trainer 信号やデータ蓄積後の再評価に向けて機構は残す。
- prior は解析的基準率で十分（クエリ不要・リークなし）。将来 results 全体の実測ベースレートへ
  差し替え可能。

## 影響
- `HorseFactors` の 6 group factor が `Option<RateTriple>`→`Option<FactorStat>`（レート+出走数）。
  predict・backtest は同じ `build_factors`/`estimate_probabilities_with_config` を共有し両経路に
  一律反映。単調性 `win ≤ place ≤ show`（ADR 0007）・市場ブレンド（#72）の挙動は不変。
- predict 本番は縮約 m=10 を既定で有効化（少データ馬の win_prob が 0→正値へ緩和）。
- Repository に `horse_recency`（既定実装は空でモック不変）と `RecencySeries`/`HorseRecencyStats`
  を追加。rdb-gateway に `races.date` 別集計クエリを実装。DB・マイグレーション変更なし。
- `analyze backtest` に `--shrinkage-m` / `--recency-half-life` を追加（パラメータスイープ用）。

## 関連
- ADR 0002（スタッツ希薄→`win_prob=0`）/ ADR 0014（None 母数除外）/ ADR 0007（単調性・欠落項除外）
- ADR 0006（バックテスト評価基盤）/ #52（校正指標）/ #31・ADR 0009（前走フォーム）/ #72（市場ブレンド）
- 設計書 `docs/specifications/probability-estimation.md`
