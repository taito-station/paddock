# ADR 0014: 実績なし factor の 0 埋め減点を None 母数除外へ統一 (Issue #81)

## ステータス
承認済み

## コンテキスト
確率推定の `raw_score` は「欠落項を母数から除外して減点しない」重み付き平均（ADR 0007/0008）。
ところが factor ごとに「実績なし」の扱いが**非一貫**だった:

- None 除外（`stat_to_triple_opt`、実績なしは母数から除外）: `trainer_surface`(#74) / `horse_track_condition`(#73)
- **0 埋め**（`stat_to_triple`、実績なし＝0 レート＝全敗扱いで母数に残り減点）: `course_gate` /
  `horse_surface` / `horse_distance` / `jockey_surface`

`horse_surface`/`horse_distance` は新馬・初距離の馬、`jockey_surface` は当該 surface 未騎乗の騎手で、
「実績なし」が 0 レート＝**不当な減点**になっていた（ADR 0011 の「実績なし ≠ 全敗」と矛盾）。
#73(PR #79) のセルフレビューで検出し別 Issue 化したもの。

## 決定

1. **4 factor すべてを `stat_to_triple_opt`（None 母数除外）に統一**する。`HorseFactors` の
   `course_gate` / `horse_surface` / `horse_distance` を `Option<RateTriple>` に変更し、
   `jockey_surface` も 0 埋めから None 除外へ揃える。`raw_score` を全項 conditional-weight に
   統一し、「実績なし」を 0 レート（全敗）と区別する。

2. **全 factor 欠落（`weight == 0.0`）の馬は `raw_score` が `0.0` を返す**（ゼロ除算 NaN の回避）。
   score 0 の馬は `normalize_to_sum` の全 0 フォールバックで均等確率に畳まれる。`course_gate` は
   コース単位統計のため通常は存在し、weight==0 は「新規コース×新馬×騎手/調教師/馬場/前走すべて
   欠落」の稀ケースのみ。

3. **採否は全期間 backtest（2025-01-01〜2026-05-31, 566 レース walk-forward）の before/after で確認**
   した。校正指標（Brier/LogLoss, #52）が確率品質＝EV/Kelly 買い目の土台であり主指標、的中率・
   回収率を従。4 factor 一括変換が両設定（model-only・本番ブレンド α=0.3）で校正・回収率とも改善
   したため、factor 単位の切り分けはせず**4 factor すべて採用**。

### backtest による before/after（566 レース、2025-01-01〜2026-05-31）

model-only（純モデル）:

| 指標 | before(0 埋め) | after(None 除外) | 差 |
|---|---|---|---|
| 単勝的中率 | 11.5% | 10.8% | -0.7 |
| 連対的中率 | 21.2% | 19.8% | -1.4 |
| 複勝的中率 | 32.3% | 29.3% | -3.0 |
| 想定回収率 | 67.4% | **71.7%** | **+4.3** |
| Brier 単/連/複 | 0.0685 / 0.1279 / 0.1745 | **0.0679 / 0.1260 / 0.1703** | 全改善 |
| LogLoss 単/連/複 | 0.5100 / 0.8475 / 1.1621 | **0.5074 / 0.8244 / 1.0670** | 全改善 |

本番設定（市場オッズ単勝ブレンド α=0.3, #72）:

| 指標 | before | after | 差 |
|---|---|---|---|
| 単勝的中率 | 36.0% | 35.7% | -0.3 |
| 連対的中率 | 51.1% | 51.4% | +0.3 |
| 複勝的中率 | 64.5% | 65.0% | +0.5 |
| 想定回収率 | 89.5% | **91.6%** | **+2.1** |
| Brier 単/連/複 | 0.0553 / 0.1205 / 0.1686 | **0.0552 / 0.1191 / 0.1653** | 全改善 |
| LogLoss 単/連/複 | 0.2010 / 0.5112 / 0.7656 | **0.2007 / 0.4917 / 0.6742** | 全改善 |

model-only の的中率は単勝 -0.7・連対 -1.4・複勝 -3.0 ポイント低下するが、これはブレンド前の
トップ選好馬ランキング（win_prob 最大馬）の粗い指標上の差。**校正（Brier/LogLoss）は両設定・全券種で
改善**し、回収率も両設定で改善（+4.3/+2.1 ポイント）。実運用の本番設定（α=0.3）では的中率も中立〜
微増で、的中率低下は表面化しない。確率品質（校正）と回収率を主指標とする評価方針に照らし、4 factor
すべての None 除外を採用する。

## 理由
- 「実績なし」を 0 レート（全敗）と同一視する 0 埋めは、新馬・初距離・未騎乗 surface の馬を構造的に
  過小評価する。None 除外はこれを母数から外し、ADR 0007（欠落項の母数除外）/ ADR 0011（実績なし
  ≠ 全敗）と全 factor で一貫させる原理的な是正である。
- backtest が校正・回収率の改善（または中立）を示し、原理的改善が実測でも裏付けられた。
- 重み定数（COURSE_GATE=2.0 等）は変更しない。本 ADR は欠落扱いの統一のみで、重みチューニングは
  別軸（[[measurement-ordering]]）。

## 影響
- `HorseFactors` の 3 フィールドが `RateTriple` → `Option<RateTriple>`。`raw_score` は全項 conditional-
  weight ＋ `weight == 0.0` フォールバック（NaN 回避）。`build_factors` は全 factor で
  `stat_to_triple_opt` を使い、0 埋め用の `stat_to_triple` は削除。
- predict・backtest は同じ `build_factors`/`estimate_probabilities` を共有するため、両経路に一律反映。
- 単調性 `win ≤ place ≤ show`（ADR 0007）・市場ブレンド（#72）の挙動は不変。
- DB・マイグレーション・Repository・gateway 変更なし。

## 関連
- ADR 0007（単調性・欠落項の母数除外）/ ADR 0011（実績なし ≠ 全敗の区別）— 本 ADR が全 factor へ拡張
- ADR 0006（バックテスト評価基盤）/ #52（校正指標）/ #72（市場オッズブレンド）
- #73(PR #79)（検出元）/ 設計書 `docs/specifications/probability-estimation.md`
