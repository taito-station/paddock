# 0047. place/show 冪変換による複勝率校正の採用（γ=2.0）

## ステータス

採用（`EstimationConfig::production()` の `placeshow_power = Some(2.0)`）

## コンテキスト

#286（再 scope）。純モデル（α=0）の校正を 71R（2026-05-30〜06-14, `scripts/predict-check/calibration.py`）で実測したところ:

- **win は健全**: ECE 1.39%・top-1 的中 31%（市場本命と同値）・モデル幅中央値 38pt（フラットでない）・大穴過大評価なし。当初 #286 が報告した「フラット・大穴過大」は現 main では非再現（#270 マージ＋データ完備で解消）。
- **show（複勝率）は校正崩れ ECE 9.3%**: 強い複勝馬を過小（45-60%帯 予測52%→実測81%）、弱い馬に複勝を配りすぎ（10-20%帯 予測17%→実測7%）＝**中央に圧縮**。
- 原因: `estimate.rs` で place/show は `normalize_to_sum(2.0/3.0)＋単調化` しか通っておらず、**win にある冪校正（`apply_win_power`, ADR 0042）に相当する校正が place/show に無い**。m=10 縮約で圧縮されたレートがそのまま出る。

**位置づけ**: `show_prob`/`place_prob` は表示値（連対率/複勝率）。EV・ポートフォリオは `simulate`（Harville）が `win_prob` から全券種確率を導くため、place/show 校正は **EV/買い目/回収率を変えない**。価値は「人間が軸/相手を選ぶときに読む複勝率の精度」向上（手動ハンデ運用に直結）。

## 決定

`apply_placeshow_power(probs, gamma)` を追加し（`apply_win_power` と同型: place を ^γ→合計2.0、show を ^γ→合計3.0 に再正規化、`win ≤ place ≤ show` を累積 max で再是正、**win は不変**）、`production()` で **γ=2.0** を適用する。win_power 適用後の place/show にかける。`default()` は `None`（後方互換）。

## 掃引（71R, place/show に γ を後掛けして ECE 測定。win 不変）

| γ | show ECE | place ECE |
|---|---|---|
| 1.00（baseline）| 9.34% | 6.61% |
| 1.25 | 8.80% | 6.02% |
| 1.50 | 7.98% | 5.11% |
| 1.75 | 7.66% | 4.28% |
| **2.00（採用）** | **7.08%** | **3.82%** |
| 2.50 | 6.41% | 4.56% |

show ECE は γ とともに単調改善するが、**place ECE は γ=2.0 で最小**（2.5 で悪化）。両者の sweet spot として γ=2.0 を採用。2.0 近傍の微細掃引（1.9〜2.3）でも place ECE は 2.0〜2.1 で底（show ECE は小標本ノイズで上下）。

## 検証（rust 実装後・71R 再生成）

- **win は byte 完全一致**（`pure_preds.tsv` vs 再生成 `pure_after.tsv` の win 列 diff なし）→ top-1 31%・win ECE 1.39% 不変。
- **production の買い目/回収率は不変**。本番の買い目生成は `build_portfolio`→`simulate`（Harville, **win 由来**）で show_prob を使わない。show_prob を使う EV 経路は `select_bets`（複勝 EV に show_prob を使用, `betting/select.rs:57`）だが、これを呼ぶのは backtest（`race/backtest.rs`）だけで、その経路は `placeshow_power: None`（off）。よって production・backtest いずれも回収率は動かない（win 不変だけからではなく、この配線から従う）。
- **show ECE 9.34%→7.04%**（掃引予測 7.08% と整合。差は Python が `%.5f` 丸め済み TSV に冪を掛けるのに対し Rust は内部フル精度で掛けるため）、place ECE 6.6%→3.8%。show の圧縮が是正され強い複勝馬に高い show が付く（60%+ 帯が populated）。
- `apply_placeshow_power` の単調性保持・ランク保存・シャープ化方向・no-op をユニットテストで担保。

## 理由

- **回収率リスクはゼロ**（win 不変由来。show/place は表示のみで EV 非関与）。show は magnitude 校正で順位を概ね保つが、place/show を別々に正規化したのち累積 max で単調再是正するため、接近した馬の前後は入れ替わりうる（厳密なランク不変ではない）。過学習リスクは単一パラメータ冪なので小。
- win に倣った最小実装で、既存の校正パイプライン（apply_win_power）と対称。

## 影響 / 留保

- 表示の連対率/複勝率が校正される。win/EV/買い目/回収率は不変。
- 71R 単一窓。magnitude 校正（単調）のため窓依存は小さいはずだが、追検証は #248 蓄積後に可能。
- place は show と同じ γ を流用（掃引で place ECE も γ=2.0 で最小と確認済み。別 γ は不要）。
- γ=2.0 のシャープ化＋各要素 `min(1.0)` cap により、突出馬の place/show が 1.0 に飽和してタイになりうる（その範囲で順位情報が落ち、場内合計が 2.0/3.0 を下回る）。`apply_win_power` と同じ既知の割り切り。表示用途のため許容。
- `analyze backtest`（win 系指標 sweep）は placeshow_power off のまま（win 非関与のため）。本校正の検証は `calibration.py`。
- **将来の foot-gun**: `select_bets`（複勝 EV に show_prob を使用）を production の買い目生成に配線する場合、本校正が複勝 EV を動かす。その時は回収率への影響を別途 backtest すること（現状 production は portfolio/simulate=win 由来で無関係）。

## 再現方法

```sh
# 入力は #252 手順で生成（/tmp/bt252）。
python3 scripts/predict-check/gen_pure_preds.py --out /tmp/bt252/pure_preds.tsv   # 純モデル(α=0)
python3 scripts/predict-check/calibration.py --pure /tmp/bt252/pure_preds.tsv \
  --placeshow-power-grid 1.0,1.25,1.5,1.75,2.0,2.5
```

**重要（掃引の再現）**: 上の掃引は **placeshow 校正を掛ける前の raw な place/show** を入力にすること。本採用後は
`production()` が `placeshow_power=2.0` を焼き込むため、現 main の `analyze predict`（=`gen_pure_preds`）が
出すのは γ=2.0 適用済みの値。これに `calibration.py` の grid が更に γ を掛けると **二重適用で掃引表が無効**
になる（γ=1.0 行も真の raw にならない）。本 ADR の掃引値は採用前バイナリ（placeshow off）で採取したもの。
再掃引する際は placeshow を off にしたビルド/設定で raw を生成してから grid を回すこと。
