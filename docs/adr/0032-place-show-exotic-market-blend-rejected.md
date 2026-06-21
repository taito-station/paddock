# ADR 0032: place/show・exotic の市場オッズブレンドを本番化しない（バックテストで棄却）

## ステータス
承認済み

## コンテキスト
確率推定の市場ブレンド（#72）は **単勝（win）のみ**で、複勝系（place/show）と exotic（馬連/馬単/三連複）の
確率は市場で校正されていない（`blend_with_market_win`, `src/domain/src/prediction/estimate.rs`）。買い目予算の
大半が複勝系・exotic に乗るため、ここが精度ボトルネックではないかという仮説で #194 を起票した。

#196（backtest 高速化, PR #203）で backtest が高速化（同 issue のベンチ母数 450R で約46分→4分）し
スイープが現実的になったため、以下を**実装して計測した**（measurement ordering 準拠: 挙動変更 →
before/after 計測）。なお 450R は #196 のベンチ母数で、本 ADR の評価窓（後述の 165R / 71R）とは別物。

- **Phase 1（place/show ブレンド）**: `show`（複勝＝3着以内）を JRA 複勝オッズの implied 確率（pool の
  overround 除去後、場内合計 3.0 へ正規化）と α ブレンドし、`place`（2着以内）は対応する単独市場オッズが
  JRA に無いため `[win, show]` にクランプして単調性（win ≤ place ≤ show）を保つ設計。win ブレンドは不変。
- **Phase 2（exotic ブレンド）**: `select_bets` の Harville 合成確率（馬連/馬単/三連複/三連単）を、各券種
  オッズの market implied（pool overround 正規化）と独立した `exotic_alpha` でブレンド。

### 計測条件
- 窓: **2026-05-30〜06-14**。評価 165R。**市場オッズ（複勝/馬連/三連複）が DB に揃うのは直近 71R のみ**
  （`race_odds` スナップショットの取得範囲）。よって place/show・exotic ブレンドが効くのはこの 71R 部分集合で、
  全165R対象の校正指標は希釈されて出る。
- DB: docker `paddock-postgres`（PG17, 2025-01〜2026-06, 4891R）。win/show 同一 α=0.3（本番設定）固定で
  exotic_alpha をスイープ。

## 決定
1. **place/show の市場ブレンドを本番化しない。** 校正は微改善するが、複勝買い目の回収率が悪化し、純益方向の
   裏付けが取れない。
2. **exotic の市場ブレンドを本番化しない。** どの `exotic_alpha` でも exotic 的中率は 0% のまま改善せず、
   かつ本番経路（`build_portfolio`）に届かない（後述）。
3. 実装コードは本体に残さない（棄却）。本 ADR に計測結果を自己完結で記録する。#194 はクローズ。
   **#195（recency 採否・win 側 α/m 再チューニング）は別タスクとして残す**（対象が異なる）。

### Phase 1（place/show）計測: 校正は微改善・複勝買い目は悪化
α=0.3 固定、165R 評価（うち市場オッズ 71R）。

| 指標 | baseline | Phase1 適用 | 差 |
|---|---|---|---|
| 単勝 Brier / LogLoss | 0.0534 / 0.1933 | 0.0534 / 0.1933 | ±0（win 不変） |
| 連対 Brier / LogLoss | 0.1053 / 0.3538 | 0.1049 / 0.3516 | 微改善 |
| 複勝 Brier / LogLoss | 0.1429 / 0.4497 | 0.1398 / 0.4406 | 改善（最大） |
| 複勝買い目 回収率（show 起点） | 79.9%（236点・的中10.2%） | **49.6%**（174点・的中8.0%） | **悪化** |

※ 「複勝買い目」は複勝券（的中＝3着以内）で、採用確率は show_prob。Phase 1 のブレンドは show_prob を直接動かすため、複勝買い目の EV 判定が変わる。
※ 校正（Brier/LogLoss）は全165R 評価。複勝買い目の点数（236/174点）は市場オッズのある 71R の curated 推奨が母集合（show ブレンドは複勝オッズのある 71R でしか効かない）。

- show を複勝オッズで下方修正した結果、校正（Brier/LogLoss）は改善（model の show 過大評価が縮む。複勝
  reliability: 予測19.7%→12.3% に対し実測10.2%→8.0%、ギャップ縮小）。
- 一方で curated 複勝買い目の回収率は 79.9%→49.6% に悪化。show_prob を下げたことで EV 通過する複勝点が
  減り、結果的に当たり筋も削れた疑い。**校正という狙いは達成するが、その先の買い目収益は悪化**。

### Phase 2（exotic）計測: α 全域で改善ゼロ
win/show α=0.3 固定、exotic_alpha をスイープ（71R, curated exotic）。

| exotic_alpha | quinella | exacta | trio | 的中率/回収率 |
|---|---|---|---|---|
| なし（従来 Harville） | 66点 | 15点 | 30点 | **全 0% / 0%** |
| 0.5 | 4点 | — | 2点 | 全 0% / 0% |
| 0.7 | 20点 | 5点 | 11点 | 全 0% / 0% |
| 0.9 | 46点 | 11点 | 22点 | 全 0% / 0% |

※ 馬単（exacta）/三連単（trifecta）も blend 対象だが、本設定（win/show α=0.3, `BettingConfig::default()`）では三連単は EV 閾値（`trifecta_ev_threshold=2.0`）を全 α で 1 点も通らず 0 点のため列を割愛。馬単も α=0.5 では 0 点で「—」。
※ 回収率は全行 0%（的中ゼロ＝払戻なし。backtest バイナリの生出力は浮動小数の負ゼロで `-0.0%` と表示されるが値は 0%）。
※ 「なし」=従来 Harville（市場フィルタ無し）の素の点数で、点数列の最多（quinella 66 点）。スイープ域を「なし→0.5→0.7→0.9」で見ると点数は谷型（quinella: 66→4→20→46）で exotic_alpha に対し単調でない（α=0.5 が最少。0.5/0.7/0.9 の範囲内だけなら単調増）。市場重みが増えるほど long-shot 組が EV 閾値（ev>1.0）を割って落ちるが、curation（券種別上限・min_kelly）との相互作用で閾値割れの量は α に厳密単調でないため。いずれにせよ全 α で的中 0%。

- **どの α でも exotic 的中率 0.0%・回収率 0%**。市場ブレンドは推奨点数を変える（市場重みが増えるほど
  long-shot 組が EV 閾値を割って減る）だけで、Harville が外す組合せを当たりに変えることはできず、
  **フィルタとしてしか働かない**。市場 implied へ寄せても勝てる exotic を生まない。
- **本番未到達**: `select_bets` は backtest 専用。本番 predict / `recommend_bets` は `build_portfolio` →
  `simulate` を使い、exotic の的中確率は win_prob から `simulate` 内で導出される。市場 exotic オッズは
  払戻倍率としてしか参照されない。よって Phase 2 を `select_bets` に入れても本番の買い目は一切変わらない。

## 理由
- **Phase 1**: #194 の主目的（複勝系の校正）は達成できるが、買い目予算が乗る複勝の回収率が悪化したため
  「校正は良くなったが負けは増える」という本末転倒になる。校正改善も 71R では小さく、薄サンプルの
  楽観アーティファクトを割り引くと本番化を正当化できない。アーティファクトの内容: 小頭数（7頭以下）では
  採用確率は show（3着以内）基準だが的中判定は複勝の払戻圏（2着以内）基準になり、この非対称で平均予測が
  実的中率を上回りやすい（`paddock-analyze backtest` の by_exotic 出力末尾に脚注として既出）。
- **Phase 2**: 市場ブレンドは確率を市場へ寄せるが、Harville がそもそも +EV と誤判定した穴目の的中を
  生み出すわけではない。α を下げる（市場重視）と点数が減るだけ、α を上げる（モデル重視）と従来 Harville
  に戻るだけで、**改善の出る α が存在しない**。加えて本番経路に届かないため、入れても益がない。
- 買い方の指針「高的中・低配当は無価値／期待値で取捨」（ローカルメモリ `feedback_betting_staking`）に
  照らしても、複勝の回収率悪化・exotic の 0% 回収はいずれも EV を改善しない方向。

## 影響
- 本体コードに変更なし（`blend_with_market_win` は win のみのまま。`select_bets` / `build_portfolio` も不変）。
- 計測のための実装（`blend_show_with_market`, `BettingConfig::exotic_alpha`, `--exotic-alpha` フラグ等）は
  恒久コードとして残さず破棄。再評価が必要なら同様に実装して回す。
- #194 はクローズ。#195 は対象が異なる（recency 採否・win 側 α/m 再チューニング）ため独立して残す。

## スコープと限界（過大結論を避けるための明記）
- **計測窓が 71R と薄い**（市場オッズの DB 保存が直近 2 週間分しか無い制約）。exotic の curated は元々
  稀な穴狙いでサンプルが特に薄く、0% 的中は窓の薄さも一因。ただし「複雑化（複勝/exotic ブレンド）を
  入れる」側に挙証責任がある中で、α 全域で改善が出ず、Phase 1 は買い目収益が悪化したため、見送りの
  根拠としては十分。
- 市場オッズ保存を広げて再計測すれば結論が変わる可能性は残るが、現状の保存範囲では本決定が妥当。
- 本 ADR は本体実装の変更を伴わない方針決定（棄却）。

## 関連
- 起票: #194（place/show・exotic 市場ブレンド）
- 前提: #196 / PR #203（backtest 高速化）でスイープが現実的になった
- 不変の本番経路: `build_portfolio`（`src/domain/src/portfolio/mod.rs`）/ `simulate`（`src/domain/src/simulation/mod.rs`）。本番 predict（`src/apps/predict`）・`recommend_bets`（`src/use-case/src/interactor/race/recommend.rs`）の双方が `build_portfolio` 経由
- 既存の win ブレンド: ADR 0027（市場ブレンドが精度レバー）/ `blend_with_market_win`
- 別タスク（残す）: #195（recency 採否・α/m 再チューニング, win 側の純 measurement）
- ローカルメモリ（リポジトリ外）: `feedback_betting_staking`（買い方方針）/ `feedback_measurement_ordering`（測定順序）
