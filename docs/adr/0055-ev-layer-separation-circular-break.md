# 0055. EV 層分離（循環断ち）— 順位付けは blended・EV は純モデル×市場odds・predict-watch を decision-support 化（採用）

## ステータス

採用（#272 Phase B で実装）。コード変更を伴う。CLAUDE.md の実買い方ルール（ROI≥100% で張る等）は本 ADR では変更しない（判断は人間に移管する設計のため、運用文言の更新は follow-up）。

## コンテキスト

predict/predict-watch は**表示も EV も市場ブレンド（α=0.2）の確率**で計算しており、EV=P_blended×odds が循環していた。単勝では厳密に

```
EV_blended = α·EV_pure + (1-α)·(1/overround)
```

（市場 implied×odds = 1/overround がレース内で一定）であり、ポートフォリオの連系・着順 EV も blended win を Harville に通すため同じ循環を含む。結果、「EV/ROI」は真の期待値でなく、+EV と出るのは `P_model·odds ≥ 1/α·(1-(1-α)/overround)`（実質 66% 超の overlay＝人気薄の較正不良が大半）に偏る。ADR 0044/#263 で「較正後 model-EV ゲートは 71R で逆予測的」と実測済み。

#272 Phase A（`analyze backtest` 4 窓 walk-forward, 2025-01〜2026-06）で確認した含意:

- 純モデルは**解像度が低い**: 1 番人気を毎窓 ~9%（実勝率 ~28%）としか出せず、フラット（≒1/頭数）で本命を見分けられない。単勝 Brier も pure > market で全窓劣る。
- 縮約（m）は犯人でない（m を外しても 1 番人気 9.2%）。フラットさは raw_score の素性設計そのもの。
- 公開データのみのモデルの「正しい確率」の天井は市場≈（ADR 0027: 市場は調教/厩舎/資金の非公開情報を織り込む）。純モデル単体で市場を超えるエッジは構造的に出ない。

## 決定

1. **循環を断つ**: EV/的中は**純モデル（α=1.0・市場非依存）× 市場odds** で計算する。市場オッズは EV 層だけに置く。
2. **順位付け（軸/相手）は blended（α=0.2）を維持する**: Phase A の通り純モデルは本命をフラットにしか出せず、軸選定は market ブレンドの方が解像度が高い（ADR 0027 と整合）。よって `build_portfolio`/`pair_ev_diagnostics` に **`rank_probs`（blended）と `ev_probs`（pure）を別々に渡す**。
3. **④ 別視点表示**: predict/predict-watch で「過去データ視点（純 P_model＋根拠＋市場 implied 比較）」と「市場 EV 視点（買い目＋純モデル EV/ROI）」を分けて出す。
4. **predict-watch を decision-support 化**: 純モデル EV/ROI は本命中心ポートフォリオでほぼ常に 100% 未満になる（純は本命を過小評価＝市場にエッジを示さない）。自動の🟢張る/⚪見送り判定をやめ、両視点を常時提示し、最終判断は人間のハンデ精査に委ねる（参考 ROI がゲート以上のときだけ 🔶 を付すが張り推奨ではない）。
5. **ROI/儲けを成功指標にしない**: 確率（特に複勝率）の正しさ＝較正/解像度を一級目標とする。#309（学習ランカー, ADR 0053）と #275/0052（α blend 廃止）は "ROI で市場超え" 基準で棄却済みだが、本件は物差しを「確率の正しさ」に変えた再定義であり、儲かる自動機械を作る話ではない。

## 理由

- **循環 EV は意味を持たない数字**だった。blended×odds は市場を一部「市場で評価」しており、+EV 判定は較正不良ゾーン（人気薄）を拾う方向に働く（ADR 0044 の逆予測性の根）。純×odds に正すと、EV は「モデルが公開データだけで市場に対し割安/割高と見るか」という coherent な信号になる。
- **順位付けまで純にすると本命選定が劣化する**（Phase A）。順位は市場情報を含む blended が良く、EV は市場と独立な pure が正しい。両者は役割が違うので分離するのが筋。
- **純 EV を自動の張り推奨に乗せない**のが誠実。天井＝市場≈で独立妙味はゼロに近く、ユーザーが実際に勝てている分の正体は手動ハンデ軸精査＝非公開情報の補完（バグでなく構造）。ツールはそれを支える decision-support に徹する。

## 実装

- `src/domain/src/portfolio/mod.rs`: `build_portfolio(rank_probs, ev_probs, odds, budget, config)` と `pair_ev_diagnostics(rank_probs, ev_probs, odds, partners)` に分離。軸/相手は `rank_axis_partners(rank_probs)`、EV/的中の `win`・`field` は `ev_probs` から。`debug_assert` で両者が同一馬集合であることを担保。
- `src/use-case/src/interactor/race/predict.rs`: `predict_race_views`（factor 収集 1 回・市場odds 1 fetch で `blended`＋`pure`＋任意の根拠を返す）を追加。`predict_race_with_diagnostics` と `recommend_bets` も dual 化。
- `src/interface/predict-format/src/lib.rs`: `format_probs_with_market`（純勝率 vs 市場 implied・差 pt）を追加。
- `src/apps/predict/src/session.rs`・`src/apps/predict-watch/src/watch.rs`: 過去データ視点／市場 EV 視点の二段表示。predict-watch は自動ゲート撤去。

## スコープ外

- raw_score のフラット原因の素性分解と isotonic 較正（#272 Phase A の follow-up）= 別フェーズ。本 ADR は「器（EV 層）を coherent にする」のみ。
- `select_bets`（backtest 計測経路, `backtest.rs`）の EV 分離 = 計測用途で production 買い目でないため対象外。
- 配分ロジック（均等割り, ADR 0046）・Kelly（#316/0054 棄却）・学習モデル（#309/0053 棄却）は不変。

## 影響

- predict/predict-watch の出力が二段（過去データ視点／市場 EV 視点）に変わる。predict-watch は自動の張る/見送り判定を出さなくなる（参考 ROI と買い目は常に提示）。
- 記録される買い目 EV（`make_bet_record`）は純モデル EV になる。
- 関連: 0027（精度の主レバーは市場ブレンド）/0044（model-EV ゲート逆予測）/0052（α blend 廃止棄却）/0053（学習モデル棄却）/0042（win-power 較正）/0047（place/show 脱圧縮）。
