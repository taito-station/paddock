---
# knowledge 規約に基づくメタデータ（docs/knowledge/README.md）。specifications はその場で
# knowledge に昇格（ADR 履歴・相互リンクを壊さないため物理移動しない）。
status: Confirmed
kind: knowledge
doc_class: [D24, D22, D19]
tags: [D24, D22, D19]
updated: "2026-08-11"
---

# 学習型モデル評価ハーネス 設計（#272 土台 / #309 受け皿）

> **現在の位置づけ（検証終了・Confirmed / 2026-07-21）**: 本ハーネス（`analyze backtest --dump-features`
> の as-of 特徴量ダンプ＋Python 評価）は **PR #310/#311/#312 で整備済み・現存**する。一方、これを受け皿と
> した **学習型 fundamental モデル路線（#272/#309）は closed**：#272・#309 とも CLOSED、学習ランカー
> （条件付きロジット/PL・非線形 GBM）は OOS で α=0.2 baseline を超えられず
> ADR 0053 で棄却された。純モデルの resolution は
> 天井（ADR 0058）、市場自体の較正補正も sub-takeout で
> exploitable でない（ADR 0059）＝「市場より上手く
> 当てる」路線は全域 closed。残るエッジは執行規律（軸ロック＋ズレ増額・ADR 0055/ADR 0060）に置く。
> したがって本書は **忠実性ハーネスの設計記録**として Confirmed（`--dump-features` 経路・as-of 忠実性
> サニティは現存の資産）だが、④ サービング以降の「学習モデル採用」節は ADR 0053 により**発動しない**
> 設計案として残す。（旧 status: Tentative は #272/#309 路線 close 後の位置づけが本文に無かったのが理由で、
> 本追記で解消。）

手作り線形 `raw_score` を学習ランカー（#309）へ置換する前提として、**リーク無しで訓練・評価でき、
任意のモデルを production の EV/買い方ロジックで対市場評価できる共通基盤**の設計。本書は #272 の土台
（分析と市場の分離・walk-forward 計測）と #309（学習モデル実装）が共有する。

## 背景：なぜ必要か（value シグナルの実証）

リーク無し `analyze backtest`・production 構成（m=10 / win_power=1.25 / place_show_power=2.0。当時は impute 無し）・4891R で
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
- production 構成は常に明示（**5 フラグ**: m=10 / win_power=1.25 / place_show_power=2.0 /
  impute_missing_factors=true / α=0.2）。`analyze backtest` の既定はどれも production と違う
  （`--impute-missing-factors` は ADR 0057 で production 側だけ true になった）。
  検証手段の書き方は [probability-estimation.md](probability-estimation.md) の「本番構成の要件（REQ）」を正とする。

### 純 Python での鏡映（α×γ 同時掃引・ADR 0045）

パラメータを振るたびに Rust binary を再実行するのは非現実的なので、**本番パイプラインを純 Python で
厳密に鏡映**し、**α=1.0 の実行 1 本から `p_model` を復元**して (α, γ) グリッドを掃引する
（`scripts/predict-check/umaren_backtest.py`）。

処理順は Rust 本番を 1:1 でなぞる（本番 α の定数は `RECOMMENDED_MARKET_BLEND_ALPHA`。ADR 0045 本文の `PRODUCTION_BLEND_ALPHA` は当時の名称で、現在その名前の定数は存在しない）:

1. `market_implied(win_odds)`: `raw = 1/odds` → `overround = Σraw`（オッズのある全頭）→ `implied = raw/overround`
2. `recompute_p_final(p_model, implied, α, γ)`: `blended = α·model + (1−α)·implied`（**implied に居る馬のみ**。
   オッズ欠落馬は model 据置）→ Σ1 正規化 → `powered = blended^γ` → Σ1 正規化
3. `recover_p_model(p_final, γ)`: α=1.0 の出力（市場補正なし＝`normalize(model^γ)`）から逆算して `p_model` を復元

**この鏡映は「速く回すための近似」ではなく厳密な等価**でなければ意味が無い。上の
「忠実性をサニティで担保」と同じ理由で、不変量テスト（`test_umaren_backtest.py`）で固定してある。

- **知見であって確定チューニングではない**。71R・赤字窓（無ゲート 71〜75% < 100%）で α・γ を同時に
  振れば**過学習が確実**で、α=0.5 で Spearman が正に転じるのも n_gate=2〜3 の小標本。本番定数
  （m=10 / α=0.2 / γ=1.25）は本 ADR では一切変更していない。確定は #248 の年間蓄積（正の母集団を
  含む窓）後に先送りしている。
- **model-EV ゲートの逆予測は ADR 0044 / 0041 / 0033 と同型**（額面 model EV の閾値抽出は較正不良ゾーンで
  ノイズを掴む）。盤面オッズ→締切ドリフトの残存相関はゲートを実際より良く見せる方向に働くので、
  それでも逆予測する以上、結論はより頑健。
- m 軸は #282 で追加して 3 軸化した（m は `p_model` に焼き込み済みのため、m の再検証には binary 再生成が要る）。
- `--p-model-dir` を指定したときだけ掃引を出力し、未指定なら既存挙動は完全に不変。

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

- Issue: #272（予測フロー再設計・親・**CLOSED**）/ #309（学習モデル実装・**CLOSED**）/ #305（純モデル value シグナル検証の提起元・クローズ済、検証は本ハーネス #272/#309 へ継承）/ #263（較正後 EV ゲートの逆予測性）
- ADR: 0027（精度のレバーは市場ブレンド）/ 0042（win_power）/ **0045（α×γ 同時再検証フレームワーク＝純 Python 鏡映・暫定知見）**/ 0047（place/show 冪変換の採用＝`place_show_power=2.0` の根拠）/ 0050（place/show raw_score 再調整の棄却）/ 0051（place/show 冪 γ の knee 確定）/ 0052（α blend 廃止の棄却）/ **0053（学習型 fundamental モデルの棄却＝#309 の結論・本路線 close）** / **0058（純 resolution 天井）** / **0059（市場較正補正の棄却＝市場側も sub-takeout で exploitable でない）** / **0055（EV 層分離＝執行エッジの土台）** / **0060（軸ロック＋ズレ増額＝残るエッジの置き所）**
- 既存: `scripts/predict-check/live_ev.py`（EV/買い方ロジック）/ `docs/specifications/backtest.md` / `probability-estimation.md`

---

## 決定ログ

<!-- この節は append-only です。既存エントリの変更・削除は CI が検出します。 -->

### ADR 0045: 確率→EV パイプライン（α×γ）の同時再検証フレームワーク（純 Python・暫定知見） (2026-06-28) — 知見／フレームワーク

#### ステータス

知見／フレームワーク（本番定数 m=10／α=0.2／γ=1.25 と CLAUDE.md は本 ADR では一切変更しない。
確定チューニングと実ルール変更は #248 の年間スナップショット蓄積〔正の母集団を含む窓〕後に先送りする）

#### コンテキスト

ADR 0044（#263）で、#246 較正（ADR 0042: 穴1着の冪変換較正）後の **model-EV ROI≥100% ゲートが
71R で逆予測的**であることが判明した（同一 baseline_pf ポートフォリオで無ゲート実 ROI 75.5% →
gate≥100% 24.5% → gate≥110% 0%、ゲートを課すほど単調悪化）。

ただしゲートに使う **最終確率は単一処理ではなく `m=10 縮約 → α=0.2 市場ブレンド → γ=1.25 冪較正 →
正規化` の合成物**であり、人気馬 EV の過大評価はこれらの相互作用で生じている公算が高い（#270）。
よって冪較正単体に閉じず、**確率→EV パイプライン全体（α・γ）を実現 ROI に対して同時再検証**し、
model EV と実現 ROI の乖離（逆予測性）が（どの係数域で）解消されるかを測るフレームワークを先行整備する。

#### 決定（知見／フレームワーク）

`scripts/predict-check/umaren_backtest.py` を拡張し、**Rust 本番パイプラインを純 Python で厳密に鏡映して
任意の (α, γ) の最終確率を再計算するフレームワーク**を追加した（#270）。binary を α・γ ごとに
再実行せず、**α=1.0 実行 1 本から p_model を復元**して全グリッドを純 Python で掃引する。

##### 再計算メソッドと p_model 復元の効率性

Rust 本番（`src/interface/rest-controller/.../race.rs`、`PRODUCTION_BLEND_ALPHA=0.2`）の処理順を鏡映する:

1. `market_implied(winodds)`: raw=1/odds、overround=Σraw（オッズのある全頭）、implied=raw/overround。
2. `recompute_p_final(p_model%, implied, α, γ)`: model=pct/100 → blended=α·model+(1−α)·implied
   （implied に居る馬のみ。オッズ欠落馬は model 据置）→ Σ1 正規化 → powered=blended^γ → Σ1 正規化 → ×100。
   α≥1.0 では (1−α)·implied 項が消え、市場補正なし（= normalize(model^γ)）。
3. `recover_p_model(p_final%, γ=1.25)`: **α=1.0 実行**は final=normalize(p_model^γ) なので、
   x=(pct/100)^(1/γ) → Σ1 正規化 → ×100 の**単一冪逆変換**で縮約済 p_model を厳密に取り出せる。

復元の効率性: 縮約（m=10）は p_model に焼き込まれており、α=1.0 では冪 1 段しか挟まらないため、
**α=1.0 の bt_pred を 1 度生成すれば、以後は binary を呼ばずに任意 (α, γ) を純 Python で再計算できる**。

##### 忠実性の検証（SANITY）

復元した p_model から再計算した (α=0.2, γ=1.25) の最終確率が、**本番 bt_pred（α=0.2 で binary が出力）
を 1 桁丸め誤差内で再現**することを 71R×全頭で確認した:

- 1005 頭で 平均絶対誤差 **0.050pt**／中央値 0.036pt／p95 0.13pt／最大 0.53pt。
- 99.3% が 0.3pt 以内、89% が 0.1pt 以内。0.3pt 超の残差は**勝率が極小の馬で 1 桁丸めが冪逆変換
  (1/1.25 乗) で増幅される**ことに起因し、構造的な順序ミスではない（純 Python 再計算は Rust 本番を
  忠実に鏡映している）。

##### 較正の信頼性（#249 枠組みと統合）

`calibration_buckets`: レースを予測 model ROI（再計算 probs での baseline_pf ポートフォリオ ROI）で
バケット分けし、各帯の **実現 ROI（Σret/Σstake）**・的中率を並べる。**「逆予測性が解消された」の定義**は
(1) Spearman(予測 ROI, レース毎実現 ROI) ≥ 0（予測が実現を正しく順位づける）かつ
(2) gate≥100% 実現 ROI ≥ 無ゲート平均（model EV ゲートが正の選別になる）の両立とする。

##### ALPHA-SIGN（符号）の訂正 ★重要

**issue #270 本文の α 記述は code と逆**。本文は「α は確率を市場へ寄せる係数。α が低い(0.2)と市場補正が
弱く…α を上げると偽 +EV が減る」とするが、**code では `blended = α·model + (1−α)·market` で α は
モデル重み**。すなわち:

- α=0.2 ＝ モデル 0.2 + **市場 0.8**＝**市場補正は強い**（本文「弱い」は誤り）。
- α を上げる ＝ **モデル重みを増やす＝市場補正を弱める**（本文の「市場へ寄せる」とは逆向き）。

issue は「α を上げる」という**操作**と「市場補正を強める」という**意図**を、誤った α 定義の下で同一視して
いるが、code 定義ではこの 2 つは**逆方向**を指す。したがって本フレームワークは予断を持たず
**α を両方向に掃引し、実現 ROI に決めさせる**（`--alpha-grid 0,0.1,0.2,0.3,0.5,1.0`）。

#### 暫定 71R 知見（2026-05-30〜06-14・過学習の明示的留保つき）

入力: production = `/tmp/bt252`（α=0.2）、p_model = `/tmp/bt270`（α=1.0 で再生成）。71R 全鞍評価。

##### (1) ADR 0044 の逆予測性を純 Python で再現（production α=0.2, γ=1.25）

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

##### (2) (α, γ) 同時掃引（n_gate=model ROI≥100% の鞍数、delta=gateROI−noGateROI）

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

##### (3) 含意

71R では**どの (α, γ) も model-EV ゲートを「正直な +EV 選別器」に戻せない**（Spearman≥0 域は n_gate
極小）。較正バケットが示すとおり、現行 (0.2, 1.25) では予測 ROI が高い鞍ほど実現 ROI が低い構造的
逆予測が残る。本フレームワークは ADR 0044 の結論を純 Python で再現・定量化し、係数掃引の土台を整えた。

#### 留保（過学習・結論の向き）

- **71R・赤字窓（無ゲート 71〜75%＜100%）で α・γ を同時チューニングすると過学習が確実**。
  α=0.5 で Spearman が正に転じるのも n_gate=2〜3 の小標本で、頑健性は無い。**暫定知見であり確定では
  ない**。確定チューニングと CLAUDE.md／本番定数の変更は **#248 の年間蓄積（正の母集団を含む窓）後**。
- **本番定数 m=10／α=0.2／γ=1.25 と CLAUDE.md は本 ADR では一切変更しない**。m（縮約）は p_model に
  焼き込み済で、本 ADR 時点の掃引対象は α×γ（m の再検証は別途 binary 再生成が必要）。
  → #282 で m 軸を追加し 3 軸化した（下記 follow-up）。本番定数の変更は依然として行わない。
- model-EV ゲートの逆予測は ADR 0044・ADR 0041・ADR 0033 と同型（額面 model EV の閾値抽出は較正不良
  ゾーンでノイズを掴む）。盤面オッズ→締切ドリフトの残存相関はゲートを実際より良く見せる方向で、
  それでも逆予測する以上、結論はより頑健。

#### 影響

- `scripts/predict-check/umaren_backtest.py`: `market_implied` / `recover_p_model` / `recompute_p_final` /
  `top1_hit` / `topk_recall` / `brier` / `spearman` / `race_winner` / `calibration_buckets` / `joint_sweep`
  を追加。`--p-model-dir`（α=1.0 の bt_pred dir）指定時のみ SANITY＋較正バケット＋(α,γ)掃引を出力し、
  未指定なら既存挙動（#250/#262/#263）は完全に不変。`--gate-grid` / `--odds-floor-grid`（ADR 0044 再現）も不変。
- `scripts/predict-check/test_umaren_backtest.py`: 上記の不変量テストを追加（standalone python3）。
- CLAUDE.md・本番定数は不変。確定較正と実ルール変更は #248 蓄積後に先送り。

#### follow-up（#282: m×α×γ への 3 軸化）

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

##### 3 軸掃引の再現方法

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

#### 再現方法

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

#### 関連

- 出自: #263／ADR 0044（較正後 model-EV ゲートの逆予測性・ルール変更保留）。
- 関連: #246・ADR 0042（冪変換較正）, ADR 0034（α 再調整の棄却）, ADR 0016/0017（縮約・recency）,
  ADR 0027（精度レバーは市場ブレンド）, #249（予測 ROI vs 実現 ROI のバケット検証）, #248（年間蓄積）,
  ADR 0040（EV ゲート閾値引き下げ棄却）。

### ADR 0047: place/show 冪変換による複勝分布の脱圧縮 (2026-06-28) — 採用

#### ステータス

採用（γ=2.0）

#### コンテキスト

確率推定は place/show スコアをレース内で合計 2.0 / 3.0 に正規化し（ADR 0007）、さらに
`win ≤ place ≤ show` を累積 max で単調化する。この「合計固定の正規化＋単調化」が分布を中央へ
**圧縮**し、本命の複勝（3着内）を大幅に過小評価・人気薄を過大評価していた（#258 / #283）。

当初 #258 は「人気薄の複勝を過小評価しているのでは」という逸話（2鞍）ベースの仮説で起票されたが、
#258 Phase 1（PR #279）で追加した人気帯別 place/show 校正を 4891R で計測したところ**仮説は反証**され、
実態は逆だった（baseline, 4891R, α=0.2・m=10・win_power=1.25）:

| 人気帯 | 予測複勝 | 実複勝 | 差 |
|---|---|---|---|
| 1番人気 | 34.8% | 59.7% | **+24.8%（大幅過小）** |
| 2-3番人気 | 25.2% | 43.2% | +18.0% |
| 4-6番人気 | 23.3% | 25.7% | +2.4% |
| 7-9番人気 | 21.3% | 12.4% | −8.9% |
| 10番人気以下 | 18.0% | 4.1% | **−13.9%（過大）** |

本命過小・人気薄過大という典型的な圧縮（中央寄せ）。改善方向は「裾を厚くする」ではなく
「**圧縮を解く（本命↑・人気薄↓）**」。

#### 決定

place/show のスコアに、正規化前に冪変換 `score'_i = score_i^γ` を掛けてから合計 2.0 / 3.0 へ
正規化する（`apply_score_power`, `EstimationConfig.place_show_power`）。`normalize_to_sum(score^γ, T)`
は `normalize(prob^γ, T)` と数学的に一致するため、**場内合計 2.0 / 3.0 を保ったまま**分布を
シャープ化（脱圧縮）できる（厳密には上限クランプ `min(1.0)` と単調化 floor `place=max(place,win)` が
発火しない範囲での恒等。γ>1 は本命スコアをシャープ化するためクランプ/floor の発火頻度を増やし、
強い本命がいるレースでは合計が 2.0 / 3.0 をわずかに下回りうる）。win の `apply_win_power`（ADR 0042）と同型だが、

- place/show は市場ブレンド対象外（ADR 0037 で棄却）なので**推定時にスコアへ適用**する（win は
  ブレンド後の確率へ適用）、
- **win_prob は一切触らない**（place/show 専用）。

backtest sweep で **γ=2.0 を採用**。`analyze backtest --place-show-power <γ>` で sweep でき、
`production()` は γ=2.0 を既定にする（`analyze predict` は production 固定で predict 側フラグ不要）。

#### バックテスト結果（2025-01-01〜2026-06-27 / 4891R, α=0.2・m=10・win_power=1.25）

##### place/show 校正（小さいほど良い）

| γ | 単勝 Brier | 連対(place) Brier | 連対 LogLoss | 複勝(show) Brier | 複勝 LogLoss |
|---|---|---|---|---|---|
| none (baseline) | 0.0543 | 0.1057 | 0.3611 | 0.1492 | 0.4724 |
| 1.25 | 0.0543 | 0.1055 | 0.3598 | 0.1483 | 0.4698 |
| 1.5 | 0.0543 | 0.1052 | 0.3587 | 0.1475 | 0.4682 |
| **2.0** | **0.0543** | **0.1049** | **0.3576** | **0.1461** | **0.4644** |

place/show の Brier・LogLoss が γ とともに**単調改善**。**単勝 Brier/LogLoss は全 γ で完全不変**
（0.0543 / 0.1954）＝設計どおり win 校正を一切汚さない。

##### 人気帯別 複勝校正（予測複勝 / 実複勝 / 差）

| 人気帯 | 実複勝 | none | γ=2.0 |
|---|---|---|---|
| 1番人気 | 59.7% | 34.8 (+24.8) | 36.8 (**+22.9**) |
| 2-3番人気 | 43.2% | 25.2 (+18.0) | 27.3 (+15.9) |
| 4-6番人気 | 25.7% | 23.3 (+2.4) | 23.7 (+2.0) |
| 7-9番人気 | 12.4% | 21.3 (−8.9) | 20.4 (−8.0) |
| 10番人気以下 | 4.1% | 18.0 (−13.9) | 16.3 (**−12.2**) |

全帯で乖離が正しい向き（本命↑・人気薄↓）に縮小。

##### 複勝（place）買い目 ROI

| γ | 点数 | 実的中率 | 回収率 |
|---|---|---|---|
| none | 239 | 9.2% | 76.7% |
| 1.25 | 240 | 9.6% | 77.3% |
| 1.5 | 238 | 9.7% | 77.9% |
| **2.0** | 239 | 10.0% | **79.2%** |

複勝買い目の回収率も単調改善（+2.5pt）。

#### 理由

- **place/show Brier/LogLoss が γ=2.0 まで単調改善**し、本 issue の核心（本命複勝の過小評価）を縮小、
  複勝買い目 ROI も改善する。
- **単勝校正・連系/着順 EV は不変**: place/show power は `win_prob` を触らないため、単勝 Brier、および
  win_prob 由来の trio（三連複）/ quinella（馬連）/ exacta（馬単）の校正・ROI はすべて baseline と同一
  （show_prob を採用確率に使うのは複勝買い目のみ）。本線の馬連・3連複戦略にノーリスクで、複勝の校正と
  買い目だけが改善する低リスク変更。
- **複勝的中率（トップ選好）は不変**（63.9%）。冪変換は単調なので argmax 不変。
- **γ=2.0 は過補正の手前で安全**: 本命の複勝乖離は +24.8 → +22.9pt と縮小するが依然大きく正（過補正＝
  負転は起きていない）。脱圧縮レバーは弱く、24.8pt の乖離の構造的主因は場内合計固定そのものにあるため
  冪変換だけでは閉じきれない。

#### 棄却・保留

- **γ≥2.5 は未掃引（棄却ではなく保留）**: none→2.0 が単調改善でまだ knee に達していないが、(1) 本命
  ギャップが +22.9pt と過補正の遥か手前で安全側、(2) レバーが弱く逓減（複勝買い目 Brier は γ=2.0 で
  0.0950→0.0964 と僅かに悪化し始める）、(3) **#286（win 側 m × recency × recent_form の joint retune）が
  place/show の素スコアを作り直す**ため、その前に γ_ps を過学習方向へ詰める意味が薄い。#286 着地後に
  joint で再評価する。**この保留作業は #290 で追跡する**（#286 着地が前提）。
- 圧縮の構造的主因（場内合計 2.0/3.0 の固定正規化）そのものの撤廃は本 ADR の範囲外（より大きな設計変更）。

#### 影響

- `EstimationConfig.place_show_power: Option<f64>` を追加。`Default` は `None`（後方互換 no-op）、
  `production()` は `RECOMMENDED_PLACE_SHOW_POWER = 2.0`。`estimate_probabilities_with_config` が
  正規化前に `apply_score_power` を place/show スコアへ適用する。
- `analyze backtest --place-show-power <γ>` を追加（未指定 no-op、再 sweep 用）。
- 回帰ガード `production_config_is_shrinkage_m10_and_recency_off` に place_show_power=2.0 の固定を追加。

#### 再現方法

```sh
BIN=./target/release/paddock-analyze
for g in "" "--place-show-power 1.25" "--place-show-power 1.5" "--place-show-power 2.0"; do
  RUST_LOG=error "$BIN" backtest --from 2025-01-01 --to 2026-06-27 \
    --blend-alpha 0.2 --shrinkage-m 10 --win-power 1.25 $g
done
# 「確率校正」の連対/複勝 Brier・LogLoss と「人気帯別 複勝圏 過小評価診断」を比較する。
```

### ADR 0050: place/show 素スコアの脱圧縮を狙った m×recency×form joint retune の棄却 (2026-06-28) — 棄却

#### ステータス

棄却（production の `EstimationConfig` は不変。place/show 校正は ADR 0047 の γ=2.0 後段変換を正とする）

#### コンテキスト

#286（再 scope）。#283（ADR 0047）は place/show の中央圧縮（本命の複勝過小評価）を後段の冪変換
γ=2.0 で是正したが、その採用時に「圧縮の根本原因は m=10 縮約が素スコアを潰しているため、win 側
m×recency×form の joint retune が素スコアを作り直せば γ への依存を下げられる（γ≥2.5 はそれまで保留）」
という仮説を残した（#286 へ委譲）。本 ADR はその仮説を 4891R で検証した結果である。

#### 検証（4891R / 2025-01-05〜2026-06-14, α=0.2・win_power=1.25, `analyze backtest`）

place/show 素スコア（`--place-show-power` 未指定＝冪変換 off）の校正を、raw レバー（m / recency /
form）を振って測定した。なお ADR 0047 の後段冪変換（place_show_power）は place/show のみを触る
設計なので win 指標はそれには不変だが、m / recency / form は `raw_score` 経由で win にも効きうる。
よって win（hold 基準）も同バッチで同時に測る（結果は下記「観察」のとおりフラット）。1番人気 複勝差は
**実測 − 予測**の率差＝percentage point（pt）。正値＝過小評価（予測 < 実測）で、0 に近いほど校正が良い。

| 構成 | 単勝 Brier | 単勝 LogLoss | 連対 Brier | 複勝 Brier | 1番人気 複勝差 |
|---|---|---|---|---|---|
| **m=10（baseline）** | 0.0543 | 0.1954 | 0.1057 | 0.1492 | **+24.8pt** |
| m=5 | 0.0544 | 0.1954 | 0.1055 | 0.1482 | +24.4pt |
| m=7 | 0.0543 | 0.1954 | 0.1056 | 0.1487 | +24.6pt |
| m=15 | 0.0543 | 0.1954 | 0.1059 | 0.1497 | +25.0pt |
| m=10, recency_half_life=60 | 0.0543 | 0.1954 | 0.1057 | 0.1490 | +24.7pt |
| m=10, recent_form_weight=0.5 | 0.0543 | 0.1954 | 0.1058 | 0.1493 | +24.9pt |
| **m=5+recency60+form0.5（最強試行）** | 0.0544 | 0.1954 | 0.1055 | 0.1481 | +24.4pt |

**観察**:

- **raw レバーは place/show をほぼ脱圧縮しない**。全 7 構成で 1番人気の複勝過小評価は **+24.4〜+25.0pt**
  （振れ幅 0.6pt）に張り付き、show Brier も 0.1481〜0.1497 で誤差レベル。最強試行（m を下げ recency と
  form を同時に強める）でも +24.8→+24.4pt（0.4pt）しか動かない。
- **win は本 sweep 範囲（m=5〜15・recency/form）でフラット**（Brier 0.0543〜0.0544 / LogLoss 0.1954）。
  これらの raw レバーは place/show を解かない一方 win も崩さない（経験的結果）。
- 対照: ADR 0047 の **γ=2.0 後段変換は単独で 1番人気 +24.8→+22.9pt（1.9pt 改善 = 最良 raw の約5倍）・
  show Brier 0.1492→0.1461（0.0031 改善 = 最良 raw の改善 0.0011 の約3倍）** と、raw レバーより明確に効く。

#### 決定

place/show 素スコアの脱圧縮を狙った **m×recency×form の joint retune は棄却**する。production の
`EstimationConfig`（m=10 / win_power=1.25 / place_show_power=2.0）は変更しない。place/show の校正は
ADR 0047 の γ=2.0 後段変換を引き続き正とする。

#### 理由

- **#283 が残した根本原因仮説（「m=10 縮約が圧縮の主因」）は反証された**。m を 5〜15 に振っても
  圧縮（本命複勝ギャップ）はほぼ動かない。圧縮の出所は縮約ではなく、`raw_score` が
  **[0,1] のレートの重み付き平均**である構造そのもの（`scoring.rs::raw_score`）。各馬のスコアが
  prior 近傍の狭い帯に収まり、`normalize_to_sum(·, 3.0)` で場内合計に割り付けると本命の複勝が
  構造的に頭打ちになる。これは縮約 m（小標本の過信抑制）や recency/form（同じ平均内の項の再重み付け）
  といった raw レバーでは解けない。
- 解くには分布を**乗法的にシャープ化**する後段変換が必要で、それが ADR 0047 の γ=2.0。raw retune は
  最良でも γ の 1/5〜1/3（複勝差で約 1/5・show Brier で約 1/3）の効果しか出ず、置き換えにも併用にも値しない。
- win は本 sweep 範囲（m=5〜15・recency/form）でほぼフラットなので「win を犠牲に place/show を取る」
  トレードオフですらない（取りに行っても place/show が得られない）。win 最適として選ばれた m=10
  （off/50 は劣化, config.rs の RECOMMENDED_SHRINKAGE_M 参照）を表示専用指標のために動かす理由は無い。

#### 影響 / 留保

- production・既存挙動とも不変。本 ADR は「現状維持の根拠」を数値で固定する記録。
- **γ≥2.5 の保留解除**: ADR 0047 が γ≥2.5 を「#286 が素スコアを作り直すため未掃引」として保留したが、
  その前提（raw retune による作り直し）は本 ADR で棄却された。よって γ≥2.5 の掃引は #286 にブロック
  されない独立の最適化課題になった（過補正リスクの確認が要るため別途。本 ADR では未実施）。`config.rs`
  の該当コメントを本 ADR 参照に更新する。
- 単一窓。指定レンジ `--from 2025-01-01 --to 2026-06-30` は DB の実レースに自動でクリップされ、
  評価対象は **4891R / 実レース初日 2025-01-05〜最終日 2026-06-14**（本文・表の窓表記はこのクリップ後の
  実レース日）。raw レバーの null 効果は構造由来で窓依存は小さいと判断。

#### 再現方法

```sh
BIN=./target/debug/paddock-analyze
# 指定レンジは DB の実レースにクリップされる（下記指定 → 実 2025-01-05〜2026-06-14 / 4891R）。
PADDOCK_DB_URL=postgres://paddock:paddock@127.0.0.1:5432/paddock \
"$BIN" backtest --from 2025-01-01 --to 2026-06-30 --blend-alpha 0.2 --win-power 1.25 \
  --shrinkage-m 5            # m を 5/7/10/15 と振る
# recency/form は単独でも測る（表の m=10+rec60 / m=10+form0.5 行）: --recency-half-life 60 を単独、
# --recent-form-weight 0.5 を単独、最後に m=5 + 両者併用（最強試行）の 3 パターン。
# place/show 素スコアを測るため --place-show-power は付けない（off）。
# 出力の「確率校正」表（単勝/連対/複勝 Brier・LogLoss）と「人気帯別 複勝圏 過小評価診断」の
# 1番人気 複勝差を比較する。
```

### ADR 0051: place/show 冪変換 γ の knee 確定 — γ=2.0 を維持 (2026-06-28) — 確定

#### ステータス

確定（`RECOMMENDED_PLACE_SHOW_POWER = 2.0` を維持。ADR 0047 を superseded せず立証・補強）

#### コンテキスト

#290（#283/#286 フォローアップ）。ADR 0047 は place/show 冪変換 `place_show_power` を γ=2.0 で採用したが、
**knee（頭打ち点）未確定**を意図的に保留した（4891R sweep で γ∈{none,1.25,1.5,2.0} が全指標単調改善のまま
2.0 で打ち切り、γ≥2.5 未掃引）。当初は「#286 が `raw_score` を作り直すと γ=2.0 の最適性が崩れる」ため #286
着地待ちとしていたが、**#286 は raw retune が place/show を脱圧縮できず棄却された（ADR 0050）＝素スコアは
不変**。よって現本番の素スコア上で γ を延長 sweep して knee を確定できる。

#### 検証（4891R / 実レース 2025-01-05〜2026-06-14, production 構成 m=10・α=0.2・win_power=1.25）

`--place-show-power` を γ∈{1.5,2.0,2.5,3.0,3.5} で sweep。win は place_show_power に不変（全 γ で単勝
Brier 0.0543）、win 由来の exotic（quinella 等）も不変（全 γ で quinella ROI 50.3%）＝sanity OK。
γ=2.0 は ADR 0047 の値（show Brier 0.1461・1番人気複勝差 +22.9pt・複勝買い目 ROI 79.2%）を完全再現。

1番人気複勝差 = 実測 − 予測（pt, 正値＝過小評価）。複勝(place)買い目 ROI は「1点100円固定・中央値近似」の参考値。

| γ | 連対 Brier | 連対 LogLoss | 複勝 Brier | 複勝 LogLoss | 1番人気複勝差 | 複勝買い目 ROI |
|---|---|---|---|---|---|---|
| 1.5 | 0.1052 | 0.3587 | 0.1475 | 0.4682 | +24.0pt | 77.9% |
| **2.0（現行）** | 0.1049 | 0.3576 | 0.1461 | 0.4644 | +22.9pt | **79.2%** |
| 2.5 | 0.1047 | 0.3569 | 0.1451 | 0.4633 | +21.4pt | 78.5% |
| 3.0 | 0.1047 | 0.3569 | 0.1445 | **0.4629** | +19.5pt | 73.6% |
| 3.5 | 0.1050 | 0.3577 | 0.1445 | 0.4662 | +17.4pt | 77.0% |

**観察**:

- **純校正（Brier/LogLoss）の knee は γ=3.0**: 複勝 LogLoss が 3.0 で**一意最小**（0.4629）→ 3.5 で悪化（0.4662）。
  連対 Brier/LogLoss は 2.5 で底に達し 3.0 まで plateau（0.1047/0.3569）、3.5 で悪化。複勝 Brier は 3.0〜3.5 で
  plateau（0.1445）。γ≥3.5 は過シャープ化で校正が崩れ始める。knee の一意な決め手は複勝 LogLoss。
- **複勝買い目 ROI は γ 全域で 77〜79% にほぼフラットで明確な net 改善が無い**: 2.0=79.2% が最高だが 2.5=78.5%・
  3.5=77.0% とノイズ幅内、3.0 のみ 73.6% に落ちる。点数も 224〜239 と γ で変動する curated 参考値（中央値近似）で、
  3.0 の谷は単一窓では noise と構造的劣化を切り分けられない。**少なくとも γ を上げて ROI が改善する兆候は無い**。
- **1番人気複勝差は γ=3.5 でも +17.4pt（過小評価のまま・過補正に達しない）**。γ では favorite の複勝過小評価を
  埋め切れない＝**構造的な床**（#286/ADR 0050 の「圧縮は raw_score の構造由来」と整合。冪変換は緩和策で根治ではない）。

#### 決定

`RECOMMENDED_PLACE_SHOW_POWER = 2.0` を**維持**する（コード値は不変、config.rs の陳腐化した「未掃引」コメントのみ
最新化）。ADR 0047 は superseded せず、本 ADR で「2.0 は恣意的でなく knee 分析上も妥当」と立証・補強する。#290 は
これをもってクローズ。

#### 理由

- **純校正の knee（3.0, 複勝 LogLoss 一意最小）まで上げる動機が無い**。3.0 への校正改善は show Brier −0.0016 と
  僅少な一方、複勝買い目 ROI は γ 全域でほぼフラット（77〜79%）で net 改善が無く、むしろ 3.0 で 73.6% に落ちる。
  place/show は decision-support（人間が読む複勝率）かつ複勝ベットの実エッジに直結するため、ROI が改善しない
  校正改善のために γ を上げる利得は無い。
- γ=2.5 は校正わずか改善・ROI 78.5%（ノイズ内）で 2.0 とほぼ等価。明確な net 改善が無いため、単一パラメータを
  不要に動かさない原則で現行 2.0 据え置きが妥当。
- favorite の複勝過小評価は γ では構造的に埋まらない（3.5 でも +17.4pt）。これは冪変換の限界であって γ を上げる
  動機にはならない（ROI 改善も無いまま床に近づくだけ）。

#### 影響 / 留保

- production・既存挙動とも不変。#283（ADR 0047）の γ=2.0 を knee 分析で立証した記録。
- 複勝買い目 ROI は curated 参考値（点数 224〜239 と γ で変動、中央値近似）でノイズを含む。3.0 の谷は単一窓では
  noise と構造的劣化を切り分けられない（3.5 で 77.0% に戻る）。確実に言えるのは「γ を上げて ROI が net 改善する
  兆候は無い」ことのみ。複数窓での ROI ノイズ再確認は #248 蓄積後に可能。
- 単一窓（4891R）。
- 本 ADR で ADR 0047 / 0050（config.rs コメント）が予告した「γ≥2.5 の独立掃引」は決着（2.0 維持）。config.rs の
  該当コメントは「未掃引」記述が本 sweep で陳腐化するため ADR 0051 参照に更新する（コード値 2.0 は不変＝doc 中心を維持）。

#### 再現方法

```sh
BIN=./target/debug/paddock-analyze
# 指定レンジは DB の実レースにクリップ（指定 → 実 2025-01-05〜2026-06-14 / 4891R）。
for g in 1.5 2.0 2.5 3.0 3.5; do
  PADDOCK_DB_URL=postgres://paddock:paddock@127.0.0.1:5432/paddock \
  "$BIN" backtest --from 2025-01-01 --to 2026-06-30 --blend-alpha 0.2 --win-power 1.25 \
    --shrinkage-m 10 --place-show-power "$g"
done
# 「確率校正」表（連対/複勝 Brier・LogLoss）、「人気帯別 複勝圏 過小評価診断」の1番人気 複勝差、
# 「買い目（curated）券種別 校正・回収率」の place 行（複勝 ROI）を γ 横断で比較する。
```

### ADR 0052: 市場オッズブレンド（α）の廃止＝純モデル化の棄却（#275 / #272 step2） (2026-06-30) — 棄却

#### ステータス

棄却（production の market blend は維持。`EstimationConfig` の α=0.2・m=10 は不変。ADR 0027 の再確認）

#### コンテキスト

#272（予測フロー再設計）は「α=0.2 の市場ブレンドが EV 計算に市場を循環させている」ことを問題視し、
ブレンド廃止＝純モデル化を検討していた（関連して #270 でも α・m・冪較正を再検証していた）。その前提として #275 で
「純モデル単体に市場非依存のシグナルがあるか／純モデルの精度水準は現行に耐えるか」を数値で確認した。

ADR 0027（#178）は既に「精度の主レバーは市場ブレンドでデータ量ではない」と結論し、その表で
model-only（≒純モデル）vs blend の比較も含んでいたが、当時の窓は **単一 72R**・α=0.3 時代と小さく、
α blend 廃止の可否を主題に据えたものではなかった。本 ADR はより大きいリーク無し窓で純モデル（α=1.0）と
現行（α=0.2）・純市場（α=0.0）を直接比較し、0027 の知見を廃止可否の判断として確定・更新する。

##### α の符号（重要）

実装は `blended = α·model + (1−α)·market`（`src/domain/src/prediction/estimate.rs:143`、`α>=1.0` で
純モデル）。**α=1.0 が純モデル / α=0.0 が純市場 / α=0.2 が現行**。#275 issue 本文・`gen_pure_preds.py`
docstring の「α=0=純モデル」は逆だった（#303 で是正済み。当該ツールは #302 で退役）。本 ADR の数値は
すべて実装基準の α。

#### 検証（890R / 2026-03-15〜06-21・`analyze backtest`・as-of 統計＝リーク無し）

`paddock-analyze backtest --blend-alpha <α>`（help に "Reproduces probability estimation with as-of
stats (no leakage)"）で α を 0.0 / 0.2 / 1.0 に振り、同一 890R を評価した。指定レンジは DB の実レース
（PDF 由来・着順あり）にクリップされ評価対象は 890R。

母数の注記: **的中率・連対・複勝・Brier は 890R 全レースが母数**。**想定回収率のみ母数が α 毎に変わる**
（「トップ選好馬の単勝に毎レース 100 円」固定の参考値で、トップ選好馬の単勝オッズが取得できたレースだけを
集計するため。α 毎にトップ選好馬が変わり、純モデル α=1.0 ではオッズ取得済みレースが 852 に減る。α=0.0/0.2
は 890）。回収率の横並び比較は母数差を含む点に留意。

| α（意味） | 単勝的中 | 連対 | 複勝的中 | 単勝 Brier | 想定回収率（母数） |
|---|---|---|---|---|---|
| 0.0 = 純市場 | 29.7% | 50.0% | 64.6% | 0.0545 | 73.1%（890） |
| 0.2 = 現行 | 29.9% | 50.1% | 64.5% | 0.0546 | 73.7%（890） |
| 1.0 = **純モデル** | **12.0%** | **22.6%** | **31.8%** | **0.0612** | 96.4%（852） |

（的中率系は全 890R 母数。Brier は小さいほど良い。）

**観察**:

- **純モデルは勝ち馬同定で市場に大きく劣る**。単勝的中 12.0% は純市場 29.7% の半分以下、複勝 31.8%
  も市場 64.6% の半分弱。単勝 Brier も 0.0612 と現行 0.0546 から悪化。同 backtest の reliability 曲線でも
  純モデルは予測確率が 20% を超える馬がほぼ無く（確率が中央付近に圧縮され突出した本命が立たない）、
  強い馬を絞り込めていない。
- **現行 α=0.2 は純市場 α=0.0 とほぼ完全一致**（単勝 29.9% vs 29.7%・複勝 64.5% vs 64.6%・Brier も
  0.0546 vs 0.0545 でほぼ同値）。20% のモデル重みは本命選択をほとんど動かしておらず、現行精度は事実上
  ほぼ市場由来。
- ADR 0027（72R・α=0.3 時代）の「model-only 単勝 11.1% → blend 31.9%」と**向き・桁が一致**し、より大きい
  リーク無し窓でも結論が再現された。

#### 決定

**市場ブレンドの廃止（純モデル化）は棄却**する。production は α=0.2 の market blend を維持し、
`EstimationConfig`（α=0.2 / m=10）を変更しない。#275 判断表の「純モデルが有意に劣る → blend を廃止せず
較正のみ修正」に確定。これにより #272（OPEN）が検討していた α blend 廃止の選択肢は本 ADR で否定され、
#272 の再設計は「blend を保ったまま EV 算出側で市場循環を断つ」方向に絞られる（#270 は close 済み）。

#### 理由

- 純モデル化は単勝的中を 29.9%→12.0% へ崩壊させ校正（Brier）も悪化させる。market blend は
  ADR 0027 の通り**精度の支柱**で、外すと本命の過小評価（拡散した純モデル分布）が露出する。
- 現行 α=0.2 ≒ 純市場 α=0.0 である以上、「blend が EV に市場を循環させる」懸念に対し純モデル化で
  応えると、循環は消えるが**予測精度ごと失う**割に合わない取引になる。循環の解消は精度を保つ別手段
  （EV 算出側の設計・較正修正）で図るべきで、blend 廃止はその手段にならない。

#### 影響 / 留保

- production・既存挙動とも不変。本 ADR は「market blend 維持」の根拠を数値で固定する記録。
- **判断軸は的中率だが、本 PJ の選択基準は EV/ROI である点の整理**（CLAUDE.md「的中率でなく期待値で
  選ぶ／高的中・低配当は無価値」）。想定回収率では純モデル 96.4% > 現行 73.7% と**逆向き**で、純モデルの
  value シグナルは未否定。ただし (1) 単勝的中 12%・<100%（依然赤字）で決定打ではない（高オッズ的中由来で
  分散が大きいと見込まれるが点推定のみで分散は未計測）、(2) 母数が 852 と他 α（890）より小さく、しかも
  トップ選好馬の単勝オッズが取れた非ランダム部分集合で選択効果が乗りうる、(3) blend を外すと精度が崩壊し
  本命過小評価が露出する。よって ROI の優位は
  「現状の純モデルを採用する根拠」にはならず、本 ADR の「廃止＝純モデル化の棄却」を覆さない。value シグナルの
  真偽切り分けは #305（#272 配下）に委ねる。
- **「純モデルの的中が低い＝モデルの作りが悪い」は部分的に正しく、改善対象**。backtest の校正診断は純モデルの
  具体的弱点を晒している（1番人気 連対を予測28-32%→実測46%と過小評価、10番人気以下を予測10.5%→実測1.9%と
  過大評価＝favorite-longshot ミスキャリブレーション。確率が中央付近に圧縮され強い馬に立たない。`raw_score` 構造由来、
  ADR 0047/0050）。一方で**公開データ（過去走）だけのモデルが本命的中率で liquid な単勝市場を上回るのは本質的に
  困難**で（市場は厩舎の自信・調教・資金流入などモデルが見られない情報を織り込む、ADR 0027）、低的中の全てが
  「作りの悪さ」ではない。本 ADR の決定は「モデルを良くするな」ではなく**「モデルが市場に伍するまで blend を
  外すな」**であり、モデル構築の改良（較正・特徴量）と value の検証は #305（#272 配下）として推進する。
- **計測経路の注意**: 本結論は `analyze backtest`（as-of）に基づく。`analyze predict` は集計統計に
  `as_of=None`（未来込み）を使うため過去レース再予想ではリークし（純モデル回収率が 295% 等の非現実値に
  なる）、`gen_pure_preds.py`/`calibration.py` の予想経由校正はリークしていた（**両ツールは #302 で退役済み**。
  リーク無しの同等校正は `analyze backtest` を使う）。本番ライブ予想は予想時点で
  未来データが DB に無いため安全。
- 単一窓（890R / 2026-03-15〜06-21）。α 自体の確定的再チューニングは複数窓 + train/validation を要し
  本 ADR では行わない（ADR 0027 決定 2・ADR 0034 を踏襲。本 ADR の主張は「廃止＝純モデル化の棄却」に限定）。

#### 再現方法

```sh
BIN=./target/release/paddock-analyze
PADDOCK_DB_URL=postgres://paddock:paddock@127.0.0.1:5432/paddock \
"$BIN" backtest --from 2026-03-15 --to 2026-06-21 --blend-alpha 0.0   # 純市場
"$BIN" backtest --from 2026-03-15 --to 2026-06-21 --blend-alpha 0.2   # 現行
"$BIN" backtest --from 2026-03-15 --to 2026-06-21 --blend-alpha 1.0   # 純モデル
# 冒頭サマリの「単勝的中率 / 複勝的中率 / 想定回収率」と「確率校正（単勝 Brier）」を比較する。
# 「分布が拡散して強い馬を絞れない」観察は出力中の「reliability 曲線（単勝・予測確率帯ごと）」で確認できる
# （純モデルは 20% 超の予測確率帯の件数がほぼ無い）。「人気帯別 複勝圏 過小評価診断」で本命過小評価も見える。
# 過去レース校正を analyze predict 経由で測るとリークする（旧 gen_pure_preds 等は #302 で退役済み）。
```

#### 関連

- Issue: #275（α blend 廃止の前提確認）/ #272（予測フロー再設計・親）/ #270（α・m・冪較正の再検証, close 済み）/
  #305（win 校正改善・純モデル value 検証）/ #302（calibration 計測の predict 経由リーク）/ #303（α 符号の意味づけ統一）
- ADR 0027（精度のレバーは市場ブレンド）/ ADR 0031（API の blend-alpha 既定）/ ADR 0034（α 再調整の棄却）
  / ADR 0047・0050（place/show の中央圧縮＝raw_score 構造由来の校正課題）

### ADR 0053: 学習型 fundamental モデル（条件付きロジット/PL・非線形 GBM）への raw_score 置換の棄却（#309 / #272 Phase B） (2026-06-30) — 棄却

#### ステータス

棄却（production の `raw_score`＋α=0.2 市場ブレンドを維持。学習モデルへの置換は見送り。ADR 0027/0052 の再確認）

#### コンテキスト

#309（#272 配下）は、手作りの線形レート加重平均である `raw_score`（`src/domain/src/prediction/scoring.rs`）を
データ駆動の学習ランカーへ置換し、「市場との意味ある食い違い＝エッジ」を出せるかを検証する issue。残る本質
レバーは（ADR 0027 の通りデータ量でなく）**モデルクラス**との仮説で、まず 1 手法を小さく検証し、勝てなければ
素直に棄却 ADR を残す方針だった。

前提として #272 Phase A で忠実性ハーネス（`analyze backtest --dump-features` の as-of ダンプ＋Python 評価が
backtest と一致することを担保、PR #310/#311/#312）を整備済み。本 ADR はそのダンプを入力に、学習モデルを
walk-forward で訓練し α=0.2 baseline・純市場と out-of-sample 比較した結果を記録する。

##### リーク防止

特徴量は `analyze backtest`（help: "Reproduces probability estimation with as-of stats (no leakage)"）の
as-of ダンプ（統計は予測対象日 `< D`）。さらに日付分割で**訓練は予測窓より前の日付のみ**（expanding window）に
限定する。production 構成（m=10 / win_power=1.25 / place_show_power=2.0 / α=0.2）で全期間ダンプを生成した。

#### 検証（OOS 3277R / 訓練 2025-01〜・評価 2025-07〜2026-06・月次 expanding walk-forward）

全期間ダンプ（4891R / 68,148 出走馬 / 2025-01-05〜2026-06-14）を、`date < cutoff` で訓練し前方 1 か月を
予測する月次ローリングで OOS 予測を得た（構造的にリーク無し）。2 手法を実装し、いずれも基礎特徴量のみと
市場併載の 2 変種で baseline・純市場と比較した:

- **PL（条件付きロジット）**: レース内 softmax `P(i 1着)=softmax(β·x_i)` を winner の条件付き対数尤度で
  L2 正則化付き当てはめ（McFadden）。特徴量 = factor 勝率6＋signal3（標準化、欠落は訓練 fold 平均/中立補完）。
- **HGB（非線形 GBM 木）**: ヒストグラム勾配ブースティング（sklearn `HistGradientBoostingClassifier`）。
  特徴量 = 上記9＋factor 出走数 starts6（rate×starts 交互作用を木が使える、欠落は NaN のまま）。レース内正規化で
  win 確率化。LightGBM は libomp（OpenMP 共有ライブラリ）を要求し未導入環境でロードできないため、libomp 不要・
  NaN ネイティブで同じヒストグラム勾配ブースティング系の sklearn HGB を採用した。なお HGB は PL のような
  レース条件付き softmax ではなく、出走馬を pooled な per-horse 二値分類（is_winner）で学習し**学習後にレース内
  正規化**する近似（fundamental の marginal 寄与を非線形・交互作用込みで見る目的には十分）。

| モデル | 単勝 Brier | 単勝 LogLoss | flat ROI |
|---|---|---|---|
| 純市場（implied） | **0.0551** | **0.1975** | 74.7% |
| baseline（α=0.2・現行） | 0.0552 | 0.1981 | 74.7% |
| PL 基礎（fund のみ） | 0.0614 | 0.2343 | 70.0% |
| PL 市場あり（fund+mkt） | 0.0552 | 0.1977 | 75.7% |
| HGB 基礎（fund のみ） | 0.0610 | 0.2317 | 76.7% |
| HGB 市場あり（fund+mkt） | 0.0554 | 0.1988 | 74.6% |

（Brier/LogLoss は小さいほど良い・全出走馬を独立 Bernoulli とした **per-horse** スコア（race-level の
`-log p_winner` ではない）で全モデル共通母数のため比較は公平。flat ROI は「トップ選好馬の単勝 100 円」固定の
**総払戻倍率／賭けレース数＝粗の払戻率**（net ROI ではない）。PL は L2=1/10/100 で同結論＝正則化に頑健。）

**観察**:

- **市場を入れると学習モデルは「市場をほぼ再現し fundamental を無視」する。** PL 市場ありの係数は
  `log_market_implied = +1.04`（≈市場そのまま）に対し**全 fundamental 係数が ±0.05 未満に崩壊**（最大でも
  jockey_recent_form の +0.043。基礎のみでは jockey_surface_win +0.34 等が効くのと対照的）（`train_pl.py`
  が main で当てはめ係数を出力する。`scripts/harness/.venv/bin/python scripts/harness/train_pl.py <dump>` の
  「学習係数」節で再現可能）。HGB 市場ありも Brier 0.0554・LogLoss 0.1988 と純市場（0.0551/0.1975）・
  baseline（0.0552/0.1981）に並ぶだけで改善しない。**線形・非線形いずれでも市場が fundamental シグナルを
  包含**しており、市場に対する marginal な情報を過去走 fundamental から取り出せていない。
  - 係数のスケール注記: fundamental は標準化済み（z-score）係数、`log_market_implied` は log-implied 生値の
    係数で、直接の絶対値比較はスケールが異なる。`β_market ≈ 1` は `softmax(log implied) = implied`＝**市場の
    完全再現**を意味する意図的設計で、その上で fundamental 係数が ±0 へ落ちる（市場再現で十分＝fundamental
    不要）という**定性**が結論の核。
- **fundamental のみのモデルは市場に校正で劣る。** PL 基礎 Brier 0.0614・HGB 基礎 0.0610 はいずれも純市場
  0.0551 より明確に悪い。HGB 基礎の flat ROI 76.7% は baseline を上回るが、**校正（Brier/LogLoss）は market に
  劣るまま**で、単一窓の flat ROI（高オッズ的中由来で高分散）であり信頼できるエッジではない。
- ADR 0027（精度の主レバーは市場ブレンド）・ADR 0052（純モデルは市場に劣る）と**向きが一致**し、モデルクラスを
  線形 PL→非線形 GBM へ広げても結論が再現された。

#### 決定

**学習型 fundamental モデル（条件付きロジット/PL・非線形 GBM）への `raw_score` 置換は棄却**する。production は
現行の `raw_score`＋α=0.2 市場ブレンドを維持し、`EstimationConfig`（α=0.2 / m=10 / 冪較正）を変更しない。
これにより #309 が掲げた「モデルクラス変更で市場にエッジを出す」仮説は本検証の範囲で否定され、#272 Phase B/C
（学習モデルのサービング）は見送る。

#### 理由

- 市場を入れた最適な学習モデルは β≈1 で**市場の再現に収束**し、fundamental の marginal 寄与が（線形でも木でも）
  ほぼゼロ。これは「市場が過去走 fundamental の情報を既に織り込む」ことの直接証拠で、ADR 0027/0052 と整合する。
- fundamental のみのモデルは市場に校正で劣り、本 PJ の選択基準（EV/ROI）でも、市場 ≧ モデルの校正である以上
  EV ゲート（モデル確率 × オッズ ≥ 1）が systematic な +EV を拾えない。flat ROI の単発の優位（HGB 基礎 76.7%）は
  校正の裏付けが無く分散と判断する。
- モデルクラス（線形 PL・非線形 GBM）という残レバーを出し切って改善しなかったため、現行構成の維持が妥当。

#### 影響 / 留保

- production・既存挙動とも不変。本 ADR は「学習モデルへ置換しない」根拠を数値で固定する記録。`scripts/harness/`
  の学習・評価コード（`train_pl.py` / `train_gbm.py`）は再検証用に残すが production 経路には接続しない。
- **検証範囲の限定**: win 段のみ（PL の place/show 整合は未実装）、単一の expanding walk-forward（PL は L2、HGB は
  既定ハイパラで頑健性を確認したが網羅的スイープは未実施）、flat top-pick ROI 中心（live_ev の EV 選抜 ROI は
  未測定だが、市場 ≧ モデル校正のため +EV を拾えない見込み）。これらは「現状の特徴量・手法では市場を超えない」
  ことの否定であり、「将来いかなる学習モデルも不可能」の証明ではない。市場が見ない情報（調教・厩舎の自信・
  資金流入など、ADR 0027）を取り込む新規特徴量が得られれば再検討の余地はある。
- value シグナル（純モデルの高 ROI、ADR 0052 留保）の真偽は本 ADR では未解決のまま。市場包含の本結果は、
  fundamental 単体の overlay が高分散ノイズである可能性を支持する側の証拠になる。
- 市場特徴量・純市場・回収率に使う `win_odds` はダンプ上のオッズ（当時 race_odds スナップショット優先・
  無ければ PDF 確定単勝）で、bet 時点より後の情報を含みうる。ただしこれは**市場側を有利にする**方向で、
  「fundamental が市場を超えない＝棄却」の結論をむしろ保守的に強める（市場を過大評価しても fundamental は
  勝てない）ため、結論の妥当性は損なわれない。
- 依存追加: 学習・評価に numpy/scipy/scikit-learn を使う venv（`scripts/harness/requirements.txt`）。忠実性
  サニティ③（`faithfulness.py`）は引き続き標準ライブラリのみ。

#### 再現方法

```sh
# 1) 全期間ダンプ（production 構成・as-of＝リーク無し）
./target/debug/paddock-analyze backtest --from 2025-01-05 --to 2026-06-14 \
  --shrinkage-m 10 --win-power 1.25 --place-show-power 2.0 --blend-alpha 0.2 \
  --dump-features scripts/harness/data/dump_full.tsv

# 2) venv 構築
python3 -m venv scripts/harness/.venv
scripts/harness/.venv/bin/pip install -r scripts/harness/requirements.txt

# 3) walk-forward 評価（PL / HGB）
scripts/harness/.venv/bin/python scripts/harness/train_pl.py  scripts/harness/data/dump_full.tsv
scripts/harness/.venv/bin/python scripts/harness/train_gbm.py scripts/harness/data/dump_full.tsv
# 出力の Brier/LogLoss/flat ROI で「市場あり ≒ 市場/baseline・fundamental は校正で劣る」を確認する。
```

#### 関連

- Issue: #309（学習型 fundamental モデル・本検証）/ #272（予測フロー再設計・親）/ #305（純モデル value 検証, close 済み）
- ADR: 0027（精度のレバーは市場ブレンド）/ 0052（α blend 廃止＝純モデル化の棄却）/ 0042（win_power）/
  0047・0050・0051（place/show の中央圧縮＝raw_score 構造由来の校正課題）
- 設計: `docs/specifications/learned-model-harness.md`（3層＋サービングのハーネス設計、Phase A=③忠実性サニティ）

### ADR 0055: EV 層分離（循環断ち）— 順位付けは blended・EV は純モデル×市場odds・predict-watch を decision-support 化（採用） (2026-06-30) — 採用

#### ステータス

採用（#272 Phase B で実装）。コード変更を伴う。CLAUDE.md の実買い方ルール（ROI≥100% で張る等）は本 ADR では変更しない（判断は人間に移管する設計のため、運用文言の更新は follow-up）。

#### コンテキスト

predict/predict-watch は**表示も EV も市場ブレンド（α=0.2）の確率**で計算しており、EV=P_blended×odds が循環していた。単勝では厳密に

```
EV_blended = α·EV_pure + (1-α)·(1/overround)
```

（市場 implied×odds = 1/overround がレース内で一定）であり、ポートフォリオの連系・着順 EV も blended win を Harville に通すため同じ循環を含む。結果、「EV/ROI」は真の期待値でなく、+EV と出るのは `P_model·odds ≥ 1/α·(1-(1-α)/overround)`（実質 66% 超の overlay＝人気薄の較正不良が大半）に偏る。ADR 0044/#263 で「較正後 model-EV ゲートは 71R で逆予測的」と実測済み。

#272 Phase A（`analyze backtest` 4 窓 walk-forward, 2025-01〜2026-06）で確認した含意:

- 純モデルは**解像度が低い**: 1 番人気を毎窓 ~9%（実勝率 ~28%）としか出せず、フラット（≒1/頭数）で本命を見分けられない。単勝 Brier も pure > market で全窓劣る。
- 縮約（m）は犯人でない（m を外しても 1 番人気 9.2%）。フラットさは raw_score の素性設計そのもの。
- 公開データのみのモデルの「正しい確率」の天井は市場≈（ADR 0027: 市場は調教/厩舎/資金の非公開情報を織り込む）。純モデル単体で市場を超えるエッジは構造的に出ない。

#### 決定

1. **循環を断つ**: EV/的中は**純モデル（α=1.0・市場非依存）× 市場odds** で計算する。市場オッズは EV 層だけに置く。
2. **順位付け（軸/相手）は blended（α=0.2）を維持する**: Phase A の通り純モデルは本命をフラットにしか出せず、軸選定は market ブレンドの方が解像度が高い（ADR 0027 と整合）。よって `build_portfolio`/`pair_ev_diagnostics` に **`rank_probs`（blended）と `ev_probs`（pure）を別々に渡す**。
3. **④ 別視点表示**: predict/predict-watch で「過去データ視点（純 P_model＋根拠＋市場 implied 比較）」と「市場 EV 視点（買い目＋純モデル EV/ROI）」を分けて出す。
4. **predict-watch を decision-support 化**: 純モデル EV/ROI は本命中心ポートフォリオでほぼ常に 100% 未満になる（純は本命を過小評価＝市場にエッジを示さない）。自動の🟢張る/⚪見送り判定をやめ、両視点を常時提示し、最終判断は人間のハンデ精査に委ねる（参考 ROI がゲート以上のときだけ 🔶 を付すが張り推奨ではない）。
5. **ROI/儲けを成功指標にしない**: 確率（特に複勝率）の正しさ＝較正/解像度を一級目標とする。#309（学習ランカー, ADR 0053）と #275/0052（α blend 廃止）は "ROI で市場超え" 基準で棄却済みだが、本件は物差しを「確率の正しさ」に変えた再定義であり、儲かる自動機械を作る話ではない。

#### 理由

- **循環 EV は意味を持たない数字**だった。blended×odds は市場を一部「市場で評価」しており、+EV 判定は較正不良ゾーン（人気薄）を拾う方向に働く（ADR 0044 の逆予測性の根）。純×odds に正すと、EV は「モデルが公開データだけで市場に対し割安/割高と見るか」という coherent な信号になる。
- **順位付けまで純にすると本命選定が劣化する**（Phase A）。順位は市場情報を含む blended が良く、EV は市場と独立な pure が正しい。両者は役割が違うので分離するのが筋。
- **純 EV を自動の張り推奨に乗せない**のが誠実。天井＝市場≈で独立妙味はゼロに近く、ユーザーが実際に勝てている分の正体は手動ハンデ軸精査＝非公開情報の補完（バグでなく構造）。ツールはそれを支える decision-support に徹する。

#### 実装

- `src/domain/src/portfolio/mod.rs`: `build_portfolio(rank_probs, ev_probs, odds, budget, config)` と `pair_ev_diagnostics(rank_probs, ev_probs, odds, partners)` に分離。軸/相手は `rank_axis_partners(rank_probs)`、EV/的中の `win`・`field` は `ev_probs` から。`debug_assert` で両者が同一馬集合であることを担保。
- `src/use-case/src/interactor/race/predict.rs`: `predict_race_views`（factor 収集 1 回・市場odds 1 fetch で `blended`＋`pure`＋任意の根拠を返す）を追加。`predict_race_with_diagnostics` と `recommend_bets` も dual 化。
- `src/interface/predict-format/src/lib.rs`: `format_probs_with_market`（純勝率 vs 市場 implied・差 pt）を追加。
- `src/apps/predict/src/session.rs`・`src/apps/predict-watch/src/watch.rs`: 過去データ視点／市場 EV 視点の二段表示。predict-watch は自動ゲート撤去。

#### スコープ外

- raw_score のフラット原因の素性分解と isotonic 較正（#272 Phase A の follow-up）= 別フェーズ。本 ADR は「器（EV 層）を coherent にする」のみ。
- `select_bets`（backtest 計測経路, `backtest.rs`）の EV 分離 = 計測用途で production 買い目でないため対象外。
- 配分ロジック（均等割り, ADR 0046）・Kelly（#316/0054 棄却）・学習モデル（#309/0053 棄却）は不変。

#### 影響

- predict/predict-watch の出力が二段（過去データ視点／市場 EV 視点）に変わる。predict-watch は自動の張る/見送り判定を出さなくなる（参考 ROI と買い目は常に提示）。
- 記録される買い目 EV（`make_bet_record`）は純モデル EV になる。
- 関連: 0027（精度の主レバーは市場ブレンド）/0044（model-EV ゲート逆予測）/0052（α blend 廃止棄却）/0053（学習モデル棄却）/0042（win-power 較正）/0047（place/show 脱圧縮）。
