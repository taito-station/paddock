# 0045. 確率→EV パイプライン（α×γ）の同時再検証フレームワーク（純 Python・暫定知見）

## ステータス

知見／フレームワーク（本番定数 m=10／α=0.2／γ=1.25 と CLAUDE.md は本 ADR では一切変更しない。
確定チューニングと実ルール変更は #248 の年間スナップショット蓄積〔正の母集団を含む窓〕後に先送りする）

## コンテキスト

ADR 0044（#263）で、#246 較正（ADR 0042: 穴1着の冪変換較正）後の **model-EV ROI≥100% ゲートが
71R で逆予測的**であることが判明した（同一 baseline_pf ポートフォリオで無ゲート実 ROI 75.5% →
gate≥100% 24.5% → gate≥110% 0%、ゲートを課すほど単調悪化）。

ただしゲートに使う **最終確率は単一処理ではなく `m=10 縮約 → α=0.2 市場ブレンド → γ=1.25 冪較正 →
正規化` の合成物**であり、人気馬 EV の過大評価はこれらの相互作用で生じている公算が高い（#270）。
よって冪較正単体に閉じず、**確率→EV パイプライン全体（α・γ）を実現 ROI に対して同時再検証**し、
model EV と実現 ROI の乖離（逆予測性）が（どの係数域で）解消されるかを測るフレームワークを先行整備する。

## 決定（知見／フレームワーク）

`scripts/predict-check/umaren_backtest.py` を拡張し、**Rust 本番パイプラインを純 Python で厳密に鏡映して
任意の (α, γ) の最終確率を再計算するフレームワーク**を追加した（#270）。binary を α・γ ごとに
再実行せず、**α=1.0 実行 1 本から p_model を復元**して全グリッドを純 Python で掃引する。

### 再計算メソッドと p_model 復元の効率性

Rust 本番（`src/interface/rest-controller/.../race.rs`、`PRODUCTION_BLEND_ALPHA=0.2`）の処理順を鏡映する:

1. `market_implied(winodds)`: raw=1/odds、overround=Σraw（オッズのある全頭）、implied=raw/overround。
2. `recompute_p_final(p_model%, implied, α, γ)`: model=pct/100 → blended=α·model+(1−α)·implied
   （implied に居る馬のみ。オッズ欠落馬は model 据置）→ Σ1 正規化 → powered=blended^γ → Σ1 正規化 → ×100。
   α≥1.0 では (1−α)·implied 項が消え、市場補正なし（= normalize(model^γ)）。
3. `recover_p_model(p_final%, γ=1.25)`: **α=1.0 実行**は final=normalize(p_model^γ) なので、
   x=(pct/100)^(1/γ) → Σ1 正規化 → ×100 の**単一冪逆変換**で縮約済 p_model を厳密に取り出せる。

復元の効率性: 縮約（m=10）は p_model に焼き込まれており、α=1.0 では冪 1 段しか挟まらないため、
**α=1.0 の bt_pred を 1 度生成すれば、以後は binary を呼ばずに任意 (α, γ) を純 Python で再計算できる**。

### 忠実性の検証（SANITY）

復元した p_model から再計算した (α=0.2, γ=1.25) の最終確率が、**本番 bt_pred（α=0.2 で binary が出力）
を 1 桁丸め誤差内で再現**することを 71R×全頭で確認した:

- 1005 頭で 平均絶対誤差 **0.050pt**／中央値 0.036pt／p95 0.13pt／最大 0.53pt。
- 99.3% が 0.3pt 以内、89% が 0.1pt 以内。0.3pt 超の残差は**勝率が極小の馬で 1 桁丸めが冪逆変換
  (1/1.25 乗) で増幅される**ことに起因し、構造的な順序ミスではない（純 Python 再計算は Rust 本番を
  忠実に鏡映している）。

### 較正の信頼性（#249 枠組みと統合）

`calibration_buckets`: レースを予測 model ROI（再計算 probs での baseline_pf ポートフォリオ ROI）で
バケット分けし、各帯の **実現 ROI（Σret/Σstake）**・的中率を並べる。**「逆予測性が解消された」の定義**は
(1) Spearman(予測 ROI, レース毎実現 ROI) ≥ 0（予測が実現を正しく順位づける）かつ
(2) gate≥100% 実現 ROI ≥ 無ゲート平均（model EV ゲートが正の選別になる）の両立とする。

### ALPHA-SIGN（符号）の訂正 ★重要

**issue #270 本文の α 記述は code と逆**。本文は「α は確率を市場へ寄せる係数。α が低い(0.2)と市場補正が
弱く…α を上げると偽 +EV が減る」とするが、**code では `blended = α·model + (1−α)·market` で α は
モデル重み**。すなわち:

- α=0.2 ＝ モデル 0.2 + **市場 0.8**＝**市場補正は強い**（本文「弱い」は誤り）。
- α を上げる ＝ **モデル重みを増やす＝市場補正を弱める**（本文の「市場へ寄せる」とは逆向き）。

issue は「α を上げる」という**操作**と「市場補正を強める」という**意図**を、誤った α 定義の下で同一視して
いるが、code 定義ではこの 2 つは**逆方向**を指す。したがって本フレームワークは予断を持たず
**α を両方向に掃引し、実現 ROI に決めさせる**（`--alpha-grid 0,0.1,0.2,0.3,0.5,1.0`）。

## 暫定 71R 知見（2026-05-30〜06-14・過学習の明示的留保つき）

入力: production = `/tmp/bt252`（α=0.2）、p_model = `/tmp/bt270`（α=1.0 で再生成）。71R 全鞍評価。

### (1) ADR 0044 の逆予測性を純 Python で再現（production α=0.2, γ=1.25）

| 予測 model ROI 帯 | n | 予測ROI | 実現ROI | 的中率 |
|---|---|---|---|---|
| <80% | 11 | 70.9% | **98.1%** | 45% |
| 80–90% | 23 | 84.9% | 86.9% | 57% |
| 90–100% | 25 | 95.0% | 66.9% | 52% |
| 100–110% | 7 | 102.8% | 45.5% | 29% |
| 110–120% | 4 | 115.5% | **0.0%** | 0% |
| ≥120% | 1 | 126.5% | 0.0% | 0% |

**実現 ROI は予測 ROI が上がるほど単調に下がる（98→87→67→46→0→0）**。Spearman(予測, 実現) = **−0.167**、
gate≥100% 実現 **24.5%** vs 無ゲート平均 **75.5%**（本番 bt_pred の実 probs で算出＝ADR 0044 と一致）。
較正バケットでも逆予測性が鮮明に再現される。

> 注: この (1) 較正バケットは**本番 bt_pred の実 probs**を用いる（ADR 0044 の gate_sweep と同値に固定）。
> 下の (2) 掃引は p_model から全グリッドを一様に**再計算**するため、(α=0.2,γ=1.25) 行は 26.5%/71.4% と
> ~2pt ずれる。これは復元→再計算の 1 桁丸め残差（上記 SANITY の最大 0.53pt が ROI に伝播したもの）で、
> 順序の不一致ではない。

### (2) (α, γ) 同時掃引（n_gate=model ROI≥100% の鞍数、delta=gateROI−noGateROI）

| α | γ | n_gate | gateROI | noGate | delta | Spearman | top1 | Brier |
|---|---|---|---|---|---|---|---|---|
| 0.2 | 1.25（本番）| 12 | 26.5% | 71.4% | −44.8 | −0.167 | 32% | 0.0590 |
| 0.2 | 1.10 | 2 | 98.3% | 71.3% | +27.0 | −0.153 | 32% | 0.0590 |
| 0.0 | 1.25 | 58 | 68.6% | 64.7% | +3.9 | −0.139 | 31% | 0.0601 |
| 0.3 | 1.25 | 2 | 98.3% | 77.9% | +20.4 | −0.095 | 32% | 0.0587 |
| **0.5** | **1.25** | **2** | **98.3%** | **89.6%** | **+8.7** | **+0.052** | **35%** | **0.0588** |
| 0.5 | 1.50 | 3 | 107.6% | 89.7% | +17.9 | +0.026 | 35% | 0.0580 |
| 1.0 | 1.25 | 47 | 74.9% | 79.3% | −4.4 | **−0.231** | **61%** | 0.0648 |

- **Spearman が ≥0 に転じるのは α=0.5 帯のみ**（+0.026〜+0.064）。だが**その帯の n_gate は 2〜3 鞍**で、
  delta が正でも標本が極小＝**ノイズ**。「Spearman≥0 かつ gateROI≥noGate かつ n_gate が非自明」を
  満たす (α, γ) は 71R には存在しない。
- **α を上げる＝モデル重みを増やす（市場補正を弱める）方向に較正は改善**する（−0.167 @0.2 →
  +0.05 @0.5）。これは issue の**字面の操作**「α を上げる」とは整合するが、**意図**「市場補正を強める」
  とは逆。実現 ROI は「より市場へ寄せる(低 α)」より「よりモデルへ寄せる(高 α)」を弱く支持した（要追検証）。
- **単勝精度のトレードオフ**: top1 は α=0.2 で 32%・α=1.0(純モデル)で **61%**、Brier は α=0.2 で
  **0.0590（最良）**・α=1.0 で 0.0648（最悪）。つまり**較正は単勝確率の Brier を改善する一方、
  この窓では純モデルの方が勝ち馬の順位付け(top1)に優れる**（鋭さ vs 較正の古典的トレードオフ）。
  ただし純モデル(α=1.0)は**ゲート整合性が最悪（Spearman −0.231）**で、top1 の良さは EV ゲートの
  正しさを意味しない。

### (3) 含意

71R では**どの (α, γ) も model-EV ゲートを「正直な +EV 選別器」に戻せない**（Spearman≥0 域は n_gate
極小）。較正バケットが示すとおり、現行 (0.2, 1.25) では予測 ROI が高い鞍ほど実現 ROI が低い構造的
逆予測が残る。本フレームワークは ADR 0044 の結論を純 Python で再現・定量化し、係数掃引の土台を整えた。

## 留保（過学習・結論の向き）

- **71R・赤字窓（無ゲート 71〜75%＜100%）で α・γ を同時チューニングすると過学習が確実**。
  α=0.5 で Spearman が正に転じるのも n_gate=2〜3 の小標本で、頑健性は無い。**暫定知見であり確定では
  ない**。確定チューニングと CLAUDE.md／本番定数の変更は **#248 の年間蓄積（正の母集団を含む窓）後**。
- **本番定数 m=10／α=0.2／γ=1.25 と CLAUDE.md は本 ADR では一切変更しない**。m（縮約）は p_model に
  焼き込み済で、本 ADR 時点の掃引対象は α×γ（m の再検証は別途 binary 再生成が必要）。
  → #282 で m 軸を追加し 3 軸化した（下記 follow-up）。本番定数の変更は依然として行わない。
- model-EV ゲートの逆予測は ADR 0044・ADR 0041・ADR 0033 と同型（額面 model EV の閾値抽出は較正不良
  ゾーンでノイズを掴む）。盤面オッズ→締切ドリフトの残存相関はゲートを実際より良く見せる方向で、
  それでも逆予測する以上、結論はより頑健。

## 影響

- `scripts/predict-check/umaren_backtest.py`: `market_implied` / `recover_p_model` / `recompute_p_final` /
  `top1_hit` / `topk_recall` / `brier` / `spearman` / `race_winner` / `calibration_buckets` / `joint_sweep`
  を追加。`--p-model-dir`（α=1.0 の bt_pred dir）指定時のみ SANITY＋較正バケット＋(α,γ)掃引を出力し、
  未指定なら既存挙動（#250/#262/#263）は完全に不変。`--gate-grid` / `--odds-floor-grid`（ADR 0044 再現）も不変。
- `scripts/predict-check/test_umaren_backtest.py`: 上記の不変量テストを追加（standalone python3）。
- CLAUDE.md・本番定数は不変。確定較正と実ルール変更は #248 蓄積後に先送り。

## follow-up（#282: m×α×γ への 3 軸化）

本 ADR のフレームワークは α×γ の 2 軸掃引だった（m は α=1.0 実行 1 本に焼き込み済で純 Python では
動かせない）。#282 で **m 軸を追加**し、m×α×γ の 3 軸掃引に拡張した。

- **Rust**: `analyze predict` に `--shrinkage-m` / `--win-power` を追加（本番既定 m=10 / γ=1.25 を上書き。
  本番フロー session/predict-watch/recommend は `EstimationConfig::production()` 固定で不変）。これで
  m を振った α=1.0 bt_pred を binary から再生成できる。`gen_win_backtest_data.sh` は
  `PADDOCK_BT_SHRINKAGE_M` で m を渡せる。
- **Python**: `umaren_backtest.py` に `--p-model-dir-m M:DIR`（複数指定可）を追加。各 m は縮約を変えて
  再生成した α=1.0 bt_pred dir を与える。`joint_sweep_m` が m→α→γ の順で回し、出力に先頭 m 列を足す。
  各 (α,γ) の集計は既存 `joint_sweep` と `_eval_alpha_gamma` を共用し、単軸掃引の出力は不変。
  既存 `--p-model-dir`（単一・m=10 相当）は後方互換で温存する。
- **不変**: 本番定数・CLAUDE.md は #282 でも変更しない。確定 (m,α,γ) チューニングは #284（#248 の年間
  蓄積後）の役割。#282 は #284 の前提ツールを用意するだけ。

### 3 軸掃引の再現方法

```sh
# 各 m について α=1.0 bt_pred を別 WORKDIR に生成（m は binary 再生成が必須）。
# γ（win_power）は本番既定 1.25 固定で生成する（recover_p_models が γ=1.25 で逆変換するため）。
for M in 10 20 50; do
  PADDOCK_DB_URL=postgres://paddock:paddock@127.0.0.1:5432/paddock \
  PADDOCK_BT_ALPHA=1.0 PADDOCK_BT_SHRINKAGE_M=$M \
  PADDOCK_ANALYZE_BIN=/path/to/release/paddock-analyze \
    bash scripts/predict-check/gen_win_backtest_data.sh /tmp/bt_m$M
done

# m×α×γ 3 軸掃引（production 入力は #252 手順で /tmp/bt252）
python3 scripts/predict-check/umaren_backtest.py \
  --races /tmp/bt252/bt_races.tsv --pred-dir /tmp/bt252 --results-dir /tmp/bt252 \
  --exotic-odds /tmp/bt252/bt_exotic_odds.tsv --winodds /tmp/bt252/bt_winodds.tsv \
  --p-model-dir-m 10:/tmp/bt_m10 --p-model-dir-m 20:/tmp/bt_m20 --p-model-dir-m 50:/tmp/bt_m50
```

## 再現方法

```sh
# 1. α=1.0 の bt_pred を再生成（p_model 復元用）。production(α=0.2) は #252 手順で /tmp/bt252。
PADDOCK_DB_URL=postgres://paddock:paddock@127.0.0.1:5432/paddock \
PADDOCK_BT_ALPHA=1.0 PADDOCK_ANALYZE_BIN=/path/to/release/paddock-analyze \
  bash scripts/predict-check/gen_win_backtest_data.sh /tmp/bt270

# 2. SANITY + 較正バケット(α=0.2,γ=1.25) + (α,γ)同時掃引
python3 scripts/predict-check/umaren_backtest.py \
  --races /tmp/bt252/bt_races.tsv --pred-dir /tmp/bt252 --results-dir /tmp/bt252 \
  --exotic-odds /tmp/bt252/bt_exotic_odds.tsv --winodds /tmp/bt252/bt_winodds.tsv \
  --p-model-dir /tmp/bt270
```

## 関連

- 出自: #263／ADR 0044（較正後 model-EV ゲートの逆予測性・ルール変更保留）。
- 関連: #246・ADR 0042（冪変換較正）, ADR 0034（α 再調整の棄却）, ADR 0016/0017（縮約・recency）,
  ADR 0027（精度レバーは市場ブレンド）, #249（予測 ROI vs 実現 ROI のバケット検証）, #248（年間蓄積）,
  ADR 0040（EV ゲート閾値引き下げ棄却）。
