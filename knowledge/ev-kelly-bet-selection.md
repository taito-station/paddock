---
# knowledge 規約に基づくメタデータ（knowledge/README.md）。旧 docs/specifications/ 由来で、
# ADR 0092 で knowledge/ に統合した（`kind: specification` で区別）。
status: Confirmed
kind: specification
doc_class: [D23, D22]
tags: [D23, D22]
sources:
  - qa/QA-predict-fixed-selection-776.md
distilled_from_sha: "fd355c9d"
updated: "2026-10-31"
---

# 期待値計算・買い目選択・Kelly 配分ロジック仕様書

Issue #12 対応。推定確率とオッズから期待値（EV）を計算し、馬連重視で買い目を選択、
Kelly 基準で賭け額を決定する。

> **用途の限定（#407・2026-07）**: 本仕様の `select_bets` ＋ Kelly 配分は **本番の買い目配分では使われていない**。
> 本番 `predict` の配分は `build_portfolio`（ワイド・馬連・三連複の◎軸ながし、券種予算を 100 円単位で均等配分。
> ADR 0019。券種内の均等配分は ADR 0046 で
> 確率重み化を棄却し維持）で、Kelly 配分は 71R walk-forward で回収率が現行に劣後し **棄却済み**
> （ADR 0054）。**`select_bets` は現在 backtest 評価（`analyze backtest`）でのみ現用**、
> `kelly_fraction` は本番では curation の `min_kelly` フィルタ（薄い買い目除外）にのみ用いる。本文の buy-selection /
> Kelly 賭け額ロジックはこの限定用途の仕様として読むこと。

## 概要

![期待値・Kelly配分フロー](diagrams/ev-kelly-flow.svg)

`HorseProbability[]`（Issue #11 で実装済みの確率推定結果）と `RaceOdds`（オッズスクレイパー結果）
を受け取り、EV が閾値を超える買い目を Kelly 配分付きで返す純粋関数を Domain 層に実装する。

---

## 用語定義

| フィールド名 | 日本語 | 定義 |
|------------|-------|-----|
| `ev` | 期待値 | `probability × odds`（1.0 を超えると理論的にプラス期待値） |
| `kelly_fraction` | Kelly 比率 | 総資金に対する賭け割合（0.0〜kelly_cap） |
| `ev_threshold` | EV 閾値 | これ以上の EV を持つ買い目のみ推奨候補にする（デフォルト 1.0） |
| `trifecta_ev_threshold` | 三連単 EV 閾値 | 三連単専用のより高い閾値（デフォルト 2.0） |
| `kelly_cap` | Kelly 上限 | kelly_fraction の最大値（デフォルト 0.25 = 25%） |

> 横断の用語索引は [用語集](glossary.md)（D07）。定義の正本は本書で、用語集はここを指す。

---

## 入力

| 項目 | 型 | 説明 |
|------|----|----|
| `probabilities` | `&[HorseProbability]` | 各馬の win/place/show 推定確率 |
| `odds` | `&RaceOdds` | 馬券種ごとのオッズマップ |
| `config` | `&BettingConfig` | EV 閾値・Kelly 上限などのパラメータ |

## 出力

| 項目 | 型 | 説明 |
|------|----|----|
| 戻り値 | `Vec<BettingRecommendation>` | EV 閾値を超えた買い目一覧（優先度順） |

優先度: **馬連 > 馬単 > 三連複 > 単勝 > 複勝 > 三連単**  
三連単は `EV > trifecta_ev_threshold` を満たした場合のみ候補に追加され、常に最後尾に表示される。  
同一馬券種内は EV 降順でソートする。

---

## 型定義

### BettingConfig

```rust
pub struct BettingConfig {
    pub ev_threshold: f64,
    pub trifecta_ev_threshold: f64,
    pub kelly_cap: f64,
}

impl Default for BettingConfig {
    fn default() -> Self {
        Self { ev_threshold: 1.0, trifecta_ev_threshold: 2.0, kelly_cap: 0.25 }
    }
}
```

**フィールドの推奨範囲と挙動:**
- `ev_threshold`: 推奨 `> 0.0`。`≥ 1.0` のとき EV フィルタ通過後の kelly_fraction > 0 が保証される
- `kelly_cap`: 推奨 `(0.0, 1.0]`。`kelly_fraction` は資金割合を表すため 1.0 超は全資金を超える賭けを意味し非現実的。0 以下を設定すると全候補の kelly_fraction が 0 になる
- バリデーション実装は呼び出し側の責任とし、Domain 関数内では前提違反をパニックさせない（不正値を渡した場合、関数は破壊的ではないが有用性のない結果を返す）

### BetCombination

```rust
pub enum BetCombination {
    Win(HorseNum),
    Place(HorseNum),
    Quinella(Pair),
    Exacta(OrderedPair),
    Trio(Triple),
    Trifecta(OrderedTriple),
}
```

馬券種と組み合わせを 1 つの enum で表現することで、型不一致を防ぐ。

### BettingRecommendation

```rust
pub struct BettingRecommendation {
    pub combination: BetCombination,
    pub probability: f64,
    pub odds: f64,
    pub ev: f64,
    pub kelly_fraction: f64,
}
```

---

## アルゴリズム詳細

### 1. 組み合わせ確率推定（Harville 公式）

単一馬の `win_prob`（Issue #11 実装の `HorseProbability` 型の `win_prob` / `place_prob` / `show_prob` フィールドと対応）をベースに多頭組み合わせ確率を近似する。

| 馬券種 | 確率計算式 |
|-------|----------|
| 単勝 | `win_prob[i]` |
| 複勝 | `show_prob[i]` |
| 馬連 `{a,b}` | `win[a]·win[b]/(1−win[a]) + win[b]·win[a]/(1−win[b])` （= 馬単 a→b + 馬単 b→a） |
| 馬単 `a→b` | `win[a]·win[b]/(1−win[a])` |
| 三連複 `{a,b,c}` | 全 6 順列 `(i,j,k)` の三連単確率 `P(i→j→k)` の合計（各 `{a,b,c}` の 6 通りの順列について三連単確率を計算して合算） |
| 三連単 `a→b→c` | `win[a]·win[b]/(1−win[a])·win[c]/(1−win[a]−win[b])` |

Harville 公式の前提: 1 着馬が抜けた後のフィールドで各馬が独立に競う。
精度は限定的だが、EV 計算に十分な近似値を提供する。

**除算ゼロ対策**: `1 − win[i]` が極端に小さい（win_prob ≒ 1.0）場合は分母を最小値 `1e-6` でクランプする（`f64::EPSILON` ≈ 2.2e-16 では除算結果が天文学的な値になるため実用的な下限を使用）。

#### 1.1 discounted Harville（λ 割引・#703 Phase 2）

素の Harville は**強い馬の 2・3 着確率を系統的に過大評価する**（Benter 1994 Table 9/10。
自前実測でも blended 系統の fit 窓で最上位帯 Z=−6.0（2 着段）/−10.8（3 着段）を確認）。
Lo & Bacon-Shone の割引形で補正する:

```
σ_i = win_i^λ2 / Σ_j win_j^λ2      （2 着段・場内正規化）
τ_i = win_i^λ3 / Σ_j win_j^λ3      （3 着段・場内正規化）
P(a→b→c) = win_a · σ_b/(1−σ_a) · τ_c/(1−τ_a−τ_b)
```

- 実装は `betting::HarvilleModel`（`HarvilleParams { lambda2, lambda3 }`）。
  **λ2=λ3=1.0（`IDENTITY`）は素の Harville と bit-exact に一致**する（自由関数へ委譲）。
- **λ は確率系統ごとに別**（市場の favorite-longshot バイアスが Harville バイアスを部分相殺する
  ため・Benter Note 3）。推奨定数は `harville.rs`:
  - `RECOMMENDED_HARVILLE_LAMBDA_BLENDED = { λ2: 0.90, λ3: 0.77 }` —
    **blended 確率**に対する採用値。`BettingConfig::default()` は IDENTITY のままで、
    適用は blended 確率を渡す呼び出し側の責務（`analyze backtest --blend-alpha`（α<1.0）
    指定時に bin が既定として選ぶ。α≥1.0 は no-op ブレンド＝pure なので IDENTITY のまま。
    pure 確率に blended-fit λ を当てる系統ミスマッチを防ぐため）。**α を掃引して比較する
    ときは λ を明示固定する**（α<1.0 の境界で λ 既定も切り替わり効果が混線するため）。
    fit 窓（2025 年・2,971R）MLE: λ2=0.90 [0.87,0.93] /
    λ3=0.77 [0.74,0.80]（伊藤 2010 の JRA 44,774R: 0.81/0.70 と同方向・同水準）。
  - `RECOMMENDED_HARVILLE_LAMBDA_PURE = IDENTITY` — portfolio 経路（本番買い目の
    EV・的中確率表示。ev_probs = 純モデル）の既定（`PortfolioConfig::default()`）。
    純モデルの λ̂ は 2.29/1.80 と**逆方向**（縮約で平坦化した純モデルは条件付き段も
    平坦すぎる）で、文献整合ゲートを満たさないため割引しない。
    **つまり本番 predict の買い目 EV 表示は本変更で不変**。
- λ の再推定は `scripts/predict-check/harville_lambda_fit.py`（fit 窓 MLE → eval 窓検証。
  窓は backtest.md 評価プロトコル）。backtest の掃引は
  `analyze backtest --harville-lambda2 X --harville-lambda3 Y`（対指定必須・未指定は既定値）。
- 採用の経緯・eval 窓の数表・pure 棄却の理由は
  [betting-rule-history.md](betting-rule-history.md) 決定ログ #703 が正。

### 2. EV 計算

```
ev = probability × odds
```

複勝（PlaceOdds）は `(low + high) / 2.0` を代表値として使用する。

### 3. EV フィルタ

- 三連単以外: `ev > config.ev_threshold`
- 三連単: `ev > config.trifecta_ev_threshold`

EV ≤ 閾値の組み合わせは候補から除外する（strict greater-than のため EV = 閾値ちょうども除外）。

### 4. Kelly 計算（簡易版）

```
b = odds − 1.0      # net odds (JRA オッズはグロス=払い戻し倍率のため 1 を引いてネット倍率に変換)
                    # 複勝の場合 odds = (place_odds.low + place_odds.high) / 2.0 を使用
q = 1.0 − p         # 外れ確率
f = (p × b − q) / b
kelly_fraction = clamp(f, 0.0, kelly_cap)
```

**`ev_threshold ≥ 1.0` のとき**、EV フィルタ通過後（EV > ev_threshold）は数学的に `f > 0` が保証されるため、デフォルト設定では clamp による 0.0 打ち切りは発生しない（`ev_threshold < 1.0` に設定した場合は f ≤ 0 が発生しうる）。

### 5. 優先度マッピング

```rust
fn priority(c: &BetCombination) -> u8 {
    match c {
        BetCombination::Quinella(_)  => 0,
        BetCombination::Exacta(_)    => 1,
        BetCombination::Trio(_)      => 2,
        BetCombination::Win(_)       => 3,
        BetCombination::Place(_)     => 4,
        BetCombination::Trifecta(_)  => 5,
    }
}
```

`sort_by_key(|r| (priority(&r.combination), OrderedFloat(-r.ev)))` で安定ソートする（**sort_key の値が小さいほど先に表示される**）。  
`OrderedFloat` は `ordered-float` クレート（`use ordered_float::OrderedFloat`）を使用する。`ev` フィールドは `f64` のまま保持し、ソート時のみ `OrderedFloat` でラップする（`BettingRecommendation` 構造体の変更は不要）。

---

## 実装配置

| 内容 | パス |
|------|------|
| 型定義・関数 | `src/domain/src/betting/mod.rs` |
| Domain lib re-export | `src/domain/src/lib.rs` |
| 依存クレート追加 | `src/domain/Cargo.toml` に `ordered-float` を追加 |

Domain 層に純粋関数として実装し、IO・状態なし。`PlaceOdds` 型（`low: OddsValue`, `high: OddsValue`）は既存の `src/domain/src/odds/odds_value.rs` を参照する。

---

## 制約・注意事項

- Harville 公式はあくまで近似。オッズの市場効率を考慮しないため、EV > 1.0 が実際のプラス期待値を保証しない
- 三連複は C(n,3) 通り（18 頭で 816 組み合わせ）、三連単は P(n,3) 通り（18 頭で 4896 組み合わせ）であり、どちらも O(n³)。なお三連複の計算では各組み合わせについて 6 順列の三連単確率を合算するため、実際の積算回数は三連単と同等（最大 4896 回）になる。プロファイリング未実施のため、問題が生じた場合は上位 N 頭（目安: 上位 8〜10 頭）に絞るプルーニングを検討する。EV フィルタ後の推奨数が膨大にならないよう閾値設定が重要
- `kelly_cap` のデフォルト 0.25 は Kelly 計算値をそのまま使用し、計算値が kelly_cap を超える場合のみ kelly_cap で打ち切る上限キャップ（kelly_cap 以下の計算値は縮小しない）
- `ev-kelly-flow.drawio` がマスター。SVG は drawio から生成した GitHub インライン表示用ファイルとして維持する

---

## 買い方の要件（REQ・D23）

本書は EV / Kelly の**算出**仕様だが、実際に張る買い目を決めるのは上位の買い方ルール。その要件と、
決めた ADR の対応をここに置く（**ADR は RO なので REQ-ID は knowledge 側**。規約は
[knowledge/README.md](README.md)）。

> **置き場所**: 本書は D23 の主 spec（[doc-classes.md](doc-classes.md) の割当）なのでここに置く。
> 棄却の詳細と実測表は [betting-rule-history.md](betting-rule-history.md)（D24 主）にあり、本表は**合否バーの
> 数値だけを再掲**する（バーを本文から辿らせると検証手段として成立しないため）。
>
> **現行ルールの運用指示はリポジトリルートの `CLAUDE.md`「買い方ルール」節が正**（毎セッション自動で
> 読まれることが実効性の source。[doc-classes.md](doc-classes.md) の「体系側の既知の穴」参照）。
> ここはその**根拠と検証手段**を REQ として固定する。棄却の詳細は
> [betting-rule-history.md](betting-rule-history.md)。

<!-- REQ:begin D23 -->
| REQ-ID | 要件 | 検証手段 | 出典 | status |
|---|---|---|---|---|
| REQ-D23-001 | 券種は 3 つ（ワイド / 馬連 / 3 連複）に固定する。単勝・馬単・馬連特化は追加しない | 各棄却 ADR のバックテストを再実行し、**その ADR が置いた baseline** を上回らないこと（0033 = 3 券種 79.5%・**α=0.3 時代**なので production 構成で測り直す / 0041 = **無フィルタ対照**の馬連 ◎軸 top5 78.7%〜79.0%（ゲート込みの `baseline_pf` は 13 鞍 24.5% と小標本で、馬連特化の一部 variant がこれを上回る——**バーに使わない**。0041 が退けた根拠は無フィルタ対照との比較と「>100% セルは 6〜10 鞍・的中 1 鞍のアーティファクト」という留保）/ 0043 = 馬連 top5 単独 78.7%。いずれも 71R）| ADR 0033 / 0041 / 0043 | Confirmed |
| REQ-D23-002 | 相手は 3 券種とも model top5。広げない | 既定の「相手 5 頭」は ADR 0019 が置いた設計値。**top5 超へ広げないこと**を直接測ったのは **3 連複だけ**（ADR 0030 の相手幅スイープ＝top5 / top7 / 全頭・混戦 90R で回収率が下がる）。**ワイド**の ADR 0065 が測ったのは top3 vs top5 の**絞る側**で（262R・12 開催日で有意差なし＝実装の top5 に寄せた）、上限側は 0030 を援用している。**馬連**は weight-matched な単独スイープ未実施（0019 の既定のまま）。予算非対称の参考対照は ADR 0041 にある（top5 無フィルタ 78.7〜79.0% vs 全頭 flat 66.4%・同 71R）。測り直すなら `scripts/predict-check/strategy_eval.py --partners`。3 連複の相手幅自体は `strategy_eval.py --partners` で掃引できるが、**ADR 0030 と同条件（混戦判定＋印馬ボックス併用）での再走には再実装が要る**——0030 の検証ハーネスは「恒久コードとして残さない」使い捨てで、`strategy_eval.py` に混戦レイヤーが無い | ADR 0019（既定 5 頭）/ 0041（馬連の参考対照）/ 0030（3 連複）/ 0065（ワイド） | Tentative |
| REQ-D23-003 | 各点の金額は券種予算内の 100 円単位**均等配分**（`build_portfolio` / `distribute`）。確率重み化と脚ごと最低 ¥100 の撤廃は採らない。**券種予算の既定は円建て `(馬連1500, ワイド1500, 3連複2000)`**（ADR 0080。旧既定 1:1:1 は ¥5,000 を 100 円単位で割り切れず 20% が不執行だった） | ADR 0046 の 71R 実 ROI 比較を再実行（薄い脚への少額 spread を外すと悪化すること）。配分の既定は `cargo test -p paddock-domain default_alloc_spends_the_whole_budget`（相手 5 頭の標準構成で総賭金 == ¥5,000・券種別 1500/1500/2000）と、`scripts/predict-check/gate_calibration.py --compare-alloc` の「予算執行率」100%（開催日に実測） | ADR 0046（均等割り）/ ADR 0080（券種予算の既定） | Confirmed |
| REQ-D23-004 | fractional Kelly は賭け額配分に使わない。`betting/kelly.rs` は EV 候補選抜（`min_kelly` の curation）に留める | ADR 0054 の同一土俵比較（定額 vs Kelly・71R walk-forward）を再実行し、**定額土俵で Kelly 重みが ADR 0054 当時の対照（Python `live_ev.py` のヒューリスティック＝確率重み＋最低 ¥100 の最大剰余法・ROI 75.5%・σ 92.5）を上回らないこと**（**production の配分は均等割り**＝REQ-D23-003。0054 の「現行」は当時の Python 土俵を指す）、および **bankroll 土俵で full Kelly が破産すること** | ADR 0054 | Confirmed |
| REQ-D23-005 | 混戦判定は「◎の model 勝率の 0.70 倍以上が ◎含め 4 頭以上」。**オッズ条件を併用しない** | ADR 0028 のオッズ閾値スイープを再実行（baseline を上回る閾値が無いこと） | ADR 0028 | Confirmed |
| REQ-D23-006 | `scripts/predict-check/` の Python（`live_ev.py`）を**張る買い目の配分に使わない**。オフライン EV レポート専用（配分方式の正が `build_portfolio` であることは REQ-D01-007。ここはその裏返しの禁止事項） | `build_portfolio` の単体テストと、`predict` / `predict-watch` が同一関数を通ること | ADR 0064 の追補（#346）——**0064 本体の決定は逆**（当時はライブ writer を Python `live_ev.py` に一本化するとしていた）。Rust に一本化したのは追補側 | Confirmed |
| REQ-D23-007 | `predict-watch` の買い目選定（軸・相手・混戦判定）は**朝の選定で確定し、以後オッズで動かさない**（#767）。朝の選定＝監視窓に入る前に保存された朝の単勝スナップショット（当日 JST 0 時以降で最初の単勝）で、本番の設定（予算 ¥5,000・α=0.2）で組んだ選定。predict-watch が計算して `morning_selections` に 1 レース 1 行・先勝ちで保存する。朝の選定が無いレースは**当日の初回スイープ**（そのレースが監視窓〈既定 40 分〉に入った最初のスイープ。起動時点で既に窓内なら起動直後のスイープ）で確定する。初回スイープで固定した後に朝の選定ができても入れ替えない。固定した相手が取消なら落とすが**ライブ順位で補充しない**（点数が減る）。固定の優先順は 朝の選定（記録◎ は組むときに軸へ強制し、保存した後に入った記録◎ は使わない）→ その日の初回スイープ（そのスイープで読んだ記録◎ を軸に反映済み）→ 記録◎ の軸だけ → 固定なし。盤（`/races/{id}/board`）と対話の `predict`（対話・`--skip-all`・`--overview`。#776）も 朝の選定 → 記録◎ の軸だけ → 固定なし で買い目を組む（同じ関数 `board_fixed_selection`）。`/recommendations` は固定しない | `cargo test -p paddock-domain` の `pinned_selection_survives_market_movement_while_roi_moves`（オッズを差し替えた 2 スイープで選定が一致し ROI は動く／固定しなければ動く）・`forced_partners_drops_scratched_without_backfill`、`cargo test -p paddock-use-case --lib morning`（優先順・初回スイープの後に保存された朝の選定で入れ替えないこと）、`cargo test -p paddock-use-case --test test_predict_race` の `snapshot_saved_after_window_entry_is_not_morning`、`cargo test -p rdb-gateway --test test_morning_selections` の `first_save_wins`、`cargo test -p rdb-gateway --test test_live_ev_persistence` の `pins_return_the_earliest_sweep_not_the_latest`（**最古**を返すこと）、`cargo test -p predict --test overview` の `predict_portfolio_is_fixed_by_the_morning_selection`（predict の買い目の軸・相手が朝の選定になること）・`predict_portfolio_uses_recorded_axis_and_ignores_other_day_morning`（別の日付の朝の選定は使わず、記録◎ の軸だけ固定すること）、および開催日に `scripts/predict-check/gate_calibration.py` の「朝の固定」節と「軸（◎）の安定性」節が `0/N` になること（朝の軸の取消でライブの首位へ戻った外れは除き、該当行を確かめる） | ADR 0078 | Confirmed |
<!-- REQ:end D23 -->

**D01 と重複させない。** 「ROI ≥ 100% のレースだけ張る」「軸ロック＋ズレ増額」「買い目の提示形式と
配分の正」は [product-goals.md](product-goals.md) の
**REQ-D01-001 / 003 / 007** が正本で、ここでは採番し直さない（同じ要件に ID が 2 つあると、片方だけ更新されても機械検査は検出できない——
ADR 0073 / 0074 が排除したい二重管理そのものになる）。本表はその下で**買い方の具体**を決める要件に限る。


---

## 決定ログ

<!-- この節は append-only です。既存エントリの変更・削除は CI が検出します。 -->

### ADR 0019: 予算内・軸流しポートフォリオ生成器 (Issue #122 PR2) (2026-06-16) — 提案中

#### ステータス

提案中（本文が言及する production の `odds-scraper` / `UreqOddsScraper` は #287 / ADR 0048 で撤去。
オッズ供給は netkeiba の `OddsScraper` 実装に置換済み。ポートフォリオ生成器の設計自体は不変）

#### コンテキスト
predict の買い目出力は `select_bets`（ADR 0003 / #121 curation 済み）の EV 羅列で、
「全正 EV 組合せをフラットに並べる」ため、そのままでは買えない（#122）。実際の買い方
（買い方メモ: 軸＝本命を外さない・相手広く・保険のワイド・100 円単位）が encode されておらず、
PR1（#138）の戦略評価ハーネスでも買い方次第で回収率が大きく変わる（本命単勝のみ 51.7% 等）ことが
定量化された。predict 本番に「予算内の軸流しポートフォリオ」を出力する生成器が必要。

`HorseProbability`（確率推定）、`RaceOdds`、単一レース収支シミュレータ `simulation::simulate`
（ADR 由来の `EvReport`）は既に Domain 層にある。

#### 決定

1. **Domain 層に `portfolio` モジュールを新設する。**
   `src/domain/src/portfolio/mod.rs` に `PortfolioConfig` / `PortfolioBet` / `Portfolio` 型と
   `build_portfolio` 純粋関数を置く（IO なし）。`select_bets` は backtest 用に存続させる。

2. **軸流し（軸 1 頭ながし）を生成する。**
   軸 = `win_prob` 最大の馬（予想本命）、相手 = 次点 `partners` 頭。馬連・ワイドは軸-相手の K 点、
   三連複は軸＋相手 2 頭（C(K,2) 点）。ワイドを保険として常に含める。

3. **予算は per-race（`--race-budget`）で受け、100 円単位で配分する。**
   `alloc` 重み (馬連:ワイド:三連複) で券種へ予算を割り、券種内は 100 円単位均等配分。賄えない端数は
   買わない（PR1 `strategy_eval.distribute` と同じ流儀）。実上限は `min(race_budget, 残高)`。
   券種割当て段階でも 100 円単位 floor を取るため、配分・点数次第で予算の一部（最大で数百円規模）が
   恒常的に未消化になりうる（消化率を上げる余り再配分は将来課題）。

4. **期待値は `simulate` に委譲する。**
   ポートフォリオ全体の期待回収率・的中確率、および各脚の EV 倍率（ワイドのように的中確率の
   閉形式が無い券種を含む）を着順列挙の収支シミュレータで正確に算出する。
   **オッズ未取得の脚は EV 評価から除外する**（払戻を見積もれず、`odds=0` で混ぜると的中 0 の stake が
   ROI 分母を膨らませ過小評価になるため）。よってポートフォリオ ROI は「オッズ取得済みの脚」についての
   期待回収率であり、未取得脚がある場合は predict 出力にその旨を注記する。

5. **predict 本番の買い目推奨を軸流しポートフォリオに置き換える。**
   `apps/predict/src/session.rs` の `select_bets`＋`recommended_amounts`（Kelly 比例配分）を
   `build_portfolio` に置換。購入(y/e/s)・DB 記録・精算フローは生成器の買い目に乗せる。
   `recommended_amounts` は役目を終えたため削除する。

6. **ワイドのライブオッズ取得を追加する。**
   保険ワイドをオッズ/EV 込みで扱うため、`odds-scraper`（production の `UreqOddsScraper`）に
   ワイドページの取得・パース（`parse_wide`, 帯 low..high）を追加する。永続化（`race_odds`）は対応済み。

#### 理由
- Domain の純粋関数として置くことで use-case/apps から依存なく呼べ、PR1 ハーネスでも検証しやすい。
- 軸流し＋保険ワイドは買い方メモを直接 encode し、EV 羅列より「そのまま買える」出力になる。
- EV を `simulate` に一本化することで、券種ごとの的中確率近似を重複実装せず正確さを担保する。
- 既定値（相手 5 頭・配分 1:1:1）は固定だが、PR1 の `strategy_eval.py` で相手頭数・配分の感度を
  測って後追い調整できる（「効果が無ければ採用しない」）。

#### 影響・代替案
- 単勝/馬単/三連単は軸流しの対象外（predict 出力から外れる）。買い方メモが馬連/ワイド/三連複の
  ながしを中心とするため。backtest の `select_bets` には影響しない。
- 人気軸（market favorite）切替・配分の動的最適化は将来課題（既定は本命軸・固定配分）。
- 確率/EV の校正は #121 側で別途扱う（本 ADR は買い方＝馬券構成のみ）。

### ADR 0078: 買い目選定は当日の初回スイープで確定し、以後オッズで動かさない（採用） (2026-08-12) — 採用

#### ステータス

採用（`predict-watch` の実装を変更する。CLAUDE.md の買い方ルール本文と本番定数は変更しない
——本 ADR はルールを**実装が守るようにする**変更であって、ルールを変えるものではない）

#### コンテキスト

CLAUDE.md「軸ロックとズレ増額」（REQ-D01-003 / ADR 0060）は次を定めている。

> - 軸（◎と**基本の買い目構造**）は事前データで確定し、ブラさない
> - 発走直前オッズの用途は "ズレ増額" のみ。**点数（相手）は増やさない**
> - 軸フリップ禁止。オッズが動いただけでは見直さない

しかし `predict-watch` はスイープのたびに軸・相手・混戦判定を `rank_probs`（市場ブレンド α=0.2）
から選び直していた。α は**モデル重み**なので市場が 0.8——選定は事実上市場人気に追随する。

ADR 0076（#571）で参考ROIがレース選別のゲート指標として使えないと確定したため、
**残るエッジは軸ロック＋ズレ増額（ADR 0060）だけ**になった。その軸が勝手に動いているなら、
執行の規律そのものが成立しない。

##### 実測（`live_ev_snapshots`・2026-07-11〜08-09・発走前に 2 スイープ以上あった 154 レース）

| 動いたもの | レース数 | 割合 |
|---|---|---|
| 軸（◎） | 28 | 18% |
| 相手集合 | 62 | 40% |
| 混戦判定 | 3 | 2% |
| **軸は不変だが相手が動いた** | **51** | **33%** |

**終日ずっと同じ買い目だったのは約 60% しかない。** 実例は 2026-08-09 新潟11R で、
**発走 5 分前**に相手の 5 頭目が ④ → ⑩ に入れ替わっている。軸の往復（`4→9→4→9`）もある。

`board.rs` は #388（`4e84f87`）で記録◎に軸を固定済みだが、同コミットは
「既存呼び出し（predict/watch/recommend）は default() 経由で不変」と明記しており、
`predict-watch` は `forced_axis=None` のままだった。生資料:
[601-axis-flip-in-predict-watch.md](../docs-original/601-axis-flip-in-predict-watch.md)。

#### 決定

**1 レースの買い目選定（軸・相手・混戦判定）は当日の初回スイープで確定し、以後オッズで動かさない。**
オッズで動くのは EV/ROI と配分金額だけにする。

固定の優先順:

1. **記録◎**（人手予想 pad の Honmei）。CLAUDE.md「軸は事前データで確定」の本来の姿で、
   盤面 #388 と優先順が揃う。ただし pad は相手を持たないので固定できるのは軸だけ。
2. **その日の初回スイープ**（`live_ev_snapshots` の最古行）。軸・相手・混戦をまとめて固定する。
3. どちらも無ければ固定なし＝そのスイープで選定が決まり、次スイープ以降は 2 が効く。

付随する規則:

- **固定した相手が非出走（取消）なら落とすが、ライブ順位で補充しない**（点数が減る）。
  補充すると「オッズで選ばれた別の馬」が入り、固定の意味が消える。
- **固定した軸が非出走なら**ライブ首位へフォールバックする（既存 `forced_axis` の挙動を維持）。
  取消は CLAUDE.md が軸の見直しを認める「新情報」にあたる。
- 固定の適用・ライブ再計算との乖離・取消による除外・初回確定を**すべてログに出す**。
  従来のログは「固定されていない」ことすら読めなかった。

#### 理由

- **α=0.2 は市場 0.8。** 選定を `rank_probs` から毎回作り直す限り、買い目は市場と一緒に動く。
  これは設定ミスではなく構造なので、閾値や係数では直らない——固定するしかない。
- **軸だけ固定しても足りない。** 軸不変で相手だけ動いたレースが 51R（33%）ある。
  CLAUDE.md も「軸**と基本の買い目構造**」と書いており、相手と混戦配分は構造の一部。
- **固定は DB から読み戻す。** `predict-watch` は `--once`（cron）でも回り、スリープ・再起動を
  跨ぐ（#568 / ADR 0072）。in-memory ではプロセスが変わるたびに固定がリセットされ、
  その時点の市場で軸が決まり直す。既にアーカイブしている `live_ev_snapshots` を読めば
  **マイグレーション無しで**プロセス跨ぎに効き、記録済みのレースにも遡って効く。
- **EV は凍らない。** 固定するのは「どの組番を買うか」だけで、EV は毎回 `ev_probs`（純モデル）×
  その時点のオッズで計算し直す。これは ADR 0060 の「直前オッズの用途はズレ増額のみ」そのもの。

#### 却下した案

- **相手の欠員をライブ順位で補充する** — 点数は保てるが、補充された馬は「オッズで選ばれた馬」で、
  固定の目的（市場追随の遮断）を無効化する。点数が減るほうが規律に忠実。
- **軸だけ固定する（#388 と同じ範囲に留める）** — 実装は最小だが 51R（33%）が直らない。
  「買い目構造を固定した」と言えない状態を残すことになる。
- **記録◎のみを固定の材料にする** — CLAUDE.md の字義には最も忠実だが、`predictions` は
  2026-06-21 を最後に空・`predict_bets` は 0 行で、**現在の運用ではフリップが 1 件も止まらない**。
  予想セッションの記録を復活させるのは別の話で、それを前提にすると欠陥が放置される。
- **in-memory で固定を持つ** — 実装は軽いが `--once` と再起動で効かない（上記）。
- **盤面（board）の相手も同時に固定する** — 盤面は on-demand の再計算ビューで「初回スイープ」に
  相当する基準を持たない。何を基準に固定するかを別途決める必要があるため本 ADR では扱わない
  （誤っていたコメント「相手 top5 不変」だけ実態に合わせた）。

#### 影響

- `PortfolioConfig` に `forced_partners` / `forced_konsen_band` を追加（既定 `None` ＝現行挙動）。
  呼び出し側が運ぶ `PinnedSelection` も追加し、盤面は `axis_only` で軸だけ固定できる。
- `compose_portfolio` の第 4 引数が `Option<HorseNum>` から `&PinnedSelection` に変わる。
- `LiveEvRepository::find_live_ev_pins_by_date` を追加（`ROW_NUMBER` で race ごとの最古行）。
  相手・band は `slip` JSONB から SQL 側で typed に復元する（use-case を serde 非依存に保つ既存方針）。
  相手は `quinella` + `wide` の和集合なので、片方の券種が予算端数で ¥0 になっても取りこぼさない。
- `recorded_axis_of` を pub 化し、盤面と `predict-watch` で共用する。
- **マイグレーション不要**。
- **CLAUDE.md の買い方ルール本文は変更しない**（本 ADR はルールを実装が守るようにする変更）。
- ADR 0076 が「残るエッジは軸ロック＋ズレ増額だけ」と結論した直後の follow-up にあたる。

#### 検証上の留保

- **実地確認は次の開催日待ち。** 本 ADR を書いた 2026-08-12 は非開催で、`predict-watch` の
  `classify` は post_time と現在時刻で Due を判定するため、過去日を指定しても対象レースが出ない。
  機械検査は単体・統合テストで張った（下記）。開催日に
  `scripts/predict-check/gate_calibration.py` の「軸（◎）の安定性」節が `0/N` になることを確認する。
- **初回スイープの保存に失敗した日**は次のスイープが固定の基準になる（`save_live_ev_snapshot` は
  best-effort）。縮退するだけで壊れないが、その日の固定はやや遅い時点の市場を反映する。
- 記録◎の経路（優先順 1）は `predictions` が空のため**現時点では動かない**。
  予想セッションの記録を再開したときに初めて効く。

#### 再現方法

```sh
# フリップの実測（修正前の値を出す）
# → docs/docs-original/601-axis-flip-in-predict-watch.md「再現方法」の SQL

# テスト
cargo test -p paddock-domain                     # 固定の振る舞い（補充しない・ROI は動く 等）
cargo test -p predict-watch                      # 優先順（記録◎ → 初回スイープ → なし）
DATABASE_URL=postgres://paddock:paddock@127.0.0.1:5432/paddock \
  cargo test -p rdb-gateway --test test_live_ev_persistence   # 最古行を返すこと

# 開催日の実地確認
python3 scripts/predict-check/gate_calibration.py --payouts-dir <dir> --from <日> --to <日>
#   → 「=== 軸（◎）の安定性 ===」が 0/N
```

#### 関連

- 出自: #601（スイープ間で軸が黙って入れ替わる）
- 前提: ADR 0060（軸ロックとズレ増額）/ ADR 0076（参考ROIはゲート指標として使えない＝残るエッジは執行の規律）
- 関連: #388（盤面側の無言フリップ修正）/ ADR 0055（EV 層分離）/ ADR 0064（買い目伝票）/
  #568・ADR 0072（監視のスリープ耐性＝プロセス跨ぎで固定が要る理由）

### #601: ADR 0078 の実地確認——固定経路は実地で発火、固定時点は窓突入時 (2026-09-27) — 知見

#### コンテキスト

ADR 0078 は「検証上の留保」で、実地確認（開催日に `gate_calibration.py` の「軸（◎）の安定性」節が
`0/N` になること）を次の開催日に回していた。2026-09-27（4回中山9日・4回阪神9日）に `predict-watch` を
終日回し、確定払戻で精算した。当日は記録◎が無く、固定は優先順 2（その日の初回スイープ）の経路で動いた。
生資料: [601-axis-lock-field-check.md](../docs-original/601-axis-lock-field-check.md)、
蒸留: [QA-axis-lock-601.md](../qa/QA-axis-lock-601.md) Q8。

#### 決定

- **優先順 2（初回スイープ）の固定経路は実地で発火し、スイープ間の入れ替わりは 0（知見）。**
  発走前に 2 スイープ以上あった 17 レースで、軸の入れ替わりは `gate_calibration.py` で **0/17**。
  同節が測らない相手・混戦もログの `🔒` 行で 17R とも不変（slip 自体の突き合わせは未実施）。
  ADR 0078 の「実地確認は次の開催日待ち」の留保は、**優先順 2 の経路に限って**このエントリで解消する
  （ADR 0078 本文は append-only のため書き換えない）。優先順 1（記録◎）の経路は未確認のまま。
- **固定時点は「そのレースが監視窓（既定 40 分）に入った最初のスイープ」であることを明記する**
  （起動時点で既に窓内のレースは起動直後のスイープ）。本文 REQ-D23-007 と用語集「軸ロック」に反映した。
  同日 11:15 の朝の選定とは軸 2/17・相手 4/17 が異なった。
- **この固定時点が REQ-D01-003 の「軸は事前データで確定」を満たすかは未決とし、#724 の計測で判断する。**
  それまで実装は変えない。

#### 理由

- REQ-D01-003 の検証手段（軸安定性 `0/N`）が実データで一度も実行されていなかった。固定後の入れ替わり 0 は
  構造上の期待どおりで、これは統計的な効果測定ではなく「固定経路が実地で発火した」という機能確認である。
- 安定性節は発走前スイープどうしの比較なので、初回スイープより前（朝〜窓突入）の変化は測れない。
  その区間で軸が 2/17 変わった以上、固定時点の妥当性は別途測る必要がある。
- 朝の選定と固定伝票の精算差（固定伝票 61.6% / 朝の選定 104.4%）は 1 日・3 レースの差で、方向を論じる母数ではない。

#### 却下した代替案

- **固定時点を朝へ前倒しする**——1 日・3 レースの差を根拠に仕様を変えることになる。#724 の複数開催日の計測を待つ。
- **ADR 0078 の「検証上の留保」本文を書き換える**——決定ログは append-only（CI が検出する）。本エントリで部分的に解消する。

#### 影響

- コード変更なし。本文は REQ-D23-007（本ファイル）・REQ-D01-003（[product-goals.md](product-goals.md)）・
  用語集「軸ロック」を追従した。
- 計測 issue #724（朝 vs 窓突入時の固定の複数開催日比較）を起票した。
- 同日の #584（macOS 通知）は `🔔` 行 1 件の配送までは確認したが、バナー表示は未確認のまま（本エントリの対象外）。

### #724: 買い目を固める時点は回収率では差が出ない — ただし REQ-D01-003 は朝の固定と解釈し #767 で前倒しする (2026-10-05) — 採用

#### コンテキスト

上の #601 のエントリは「窓突入時（≒発走 40 分前・T-40）の固定が REQ-D01-003 の『軸は事前データで確定』を満たすかは未決とし、#724 の計測で判断する」とした。#724 で、朝の選定に固定した場合と T-40 の固定とを、10 開催日で比べた（[一次資料](../docs-original/724-fix-timing-morning-vs-t40.md)）。両時点の伝票は、本番と同じ `compose_portfolio` で再構成した（`paddock-analyze reconstruct-slips`。記録された T-40 の固定と 227R 中 202R で一致した。食い違った 25R は、勝率の差がその日の入力のずれ以内の近接した馬の入れ替わりで、入力が当時と同じ 10-03 では確率もほぼ完全に一致した）。判定の規則は計測の前に issue で固定した（[QA](../qa/QA-fix-timing-724.md)）。

#### 決定

1. **回収率では T-40 を維持する**。主判定（09-27 を除く 9 日・167R）で「朝 − T-40」の ROI の差は +0.001（95% CI [−0.203, +0.217]）、差が 0 でない日のうち朝が上は 4/8。MDE（判定と同じ標本から出した値）は対の差で ±0.30、朝と T-40 を独立とみなす保守側で ±0.40 で、それ未満の差はこの標本では検出できない。
2. **ただし REQ-D01-003 は「朝の時点で固定する」と解釈し、今の T-40 固定は満たさないとする**（ユーザー判断）。「事前データで確定」の「事前」を朝の予想の時点と読む。固定の時点を朝へ前倒しする仕組みは #767 で足す。#767 が入るまでの運用は今のまま（T-40 で固定）。

#### 理由

- 回収率の比較では、どちらの時点が良いかを判断できない（差は MDE より小さく、日別の向きも五分）。そのため時点の選択は、回収率ではなく規範（REQ-D01-003）で決める。
- 朝と T-40 で選定はよく変わる（軸 16%・相手 45% のレースで食い違う）。T-40 の選定には、直前の市場の動きが入っている。

#### 却下した代替案

- **回収率の差を根拠に前倒しする / 維持する**: 差は MDE 未満で、どちらの根拠にもならない。
- **T-40 の固定で REQ-D01-003 を満たすと解釈する**: 軸ロックの意図（直前のブレで乗り換えない）は守れているが、「事前」の解釈としてユーザーが採らなかった。
- **記録された T-40 の伝票をそのまま比べる**: #601 より前は配分が違い（#600）、比べられる期間が 5 日に縮む。両時点とも同じ経路で再構成した。

#### 影響

- 追加: `paddock-analyze reconstruct-slips`（過去の開催日の買い目を朝と T-40 で再構成する計測用の CLI）、`scripts/predict-check/fix_timing_probe.py`（精算と判定）。本番の経路は変えていない。
- 「朝」の定義が 2 つある。#724 の朝は「当日 JST 0 時以降で最初に取った単勝のスナップショット」で、盤の「朝↔現」比較（#448・`find_race_odds_morning`）の朝は「最初に全券種がそろったスナップショット」。#767 で朝の固定を入れるときに取り違えない。
- 予定: #767 で predict-watch の固定の元を朝の選定に変える。そのときに本文の REQ-D23-007 の行・`product-goals.md` の REQ-D01-003 の行（「未充足」を外し、朝の固定を測る検証手段に替える）・CLAUDE.md「軸ロックとズレ増額」・用語集「軸ロック」・keiba-start スキル（Step 3 の「ここで買い目を確定させない」を含む）を追従させ、ADR 0078 系の決定は #767 の決定ログとして新しく積む。
- 既知の制約: 対象は監視したレースだけ（1 日 12〜35R）。払戻の大きい少数のレースで差が振れる（上位 3 レースを除くと差は −0.055）。

### #767: 買い目の固定を朝の選定へ前倒しする (2026-10-05) — 採用

#### コンテキスト

上の #724 のエントリで、REQ-D01-003 の「事前データで確定」を「朝の時点で固定」と解釈し、今の初回スイープ（T-40）の固定は満たさないとした。#767 で固定の時点を朝へ前倒しする。方針は質問票で確定した（[QA](../qa/QA-morning-fix-767.md)）。

#### 決定

1. **朝の選定を作る**。朝＝当日 JST 0 時以降で最初に保存された単勝のスナップショット（#724 と同じ定義）。その盤で本番の設定（予算 ¥5,000・`RECOMMENDED_MARKET_BLEND_ALPHA`・`EstimationConfig::production`・統計は開催日より前）で `compose_portfolio` を組み、伝票から記録側と同じ規則（`PinnedSelection::from_staked_legs`）で軸・相手・混戦を読む。記録◎ があれば軸に強制して組む。
2. **predict-watch が計算し、新テーブル `morning_selections` に保存する**（1 レース 1 行・先勝ち・上書きしない）。1 スイープの中で、窓に入ったレースは評価の前に、窓の前のレースは評価の後に計算する。保存した後は DB から読み直した値を使う。
3. **窓に入った後に固定の元を入れ替えない**。朝として採るのは窓に入る時刻（発走 − `--window`）より前に保存されたスナップショットだけ（predict-watch 自身の初回スイープの保存を「朝」と取り違えない）。初回スイープで固定したレースには朝の選定を計算せず、仮に後から朝の選定ができても初回スイープを使う（計算時刻をスイープの `captured_at` で記録して比べる）。
4. **固定の優先順を 朝の選定 → その日の初回スイープ → 記録◎ の軸だけ → 固定なし にする**。記録◎ は朝の選定を組むときに軸へ強制し、**保存した後に入った記録◎ は使わない**（盤と predict-watch で同じ。PR のセルフレビューで盤と predict-watch の食い違いが見つかり、ユーザー判断で決めた）。朝の選定が無いレース（朝の単勝が窓に入った後・組めない）は初回スイープにフォールバックし、その件数をスイープごとのログに出す。
5. **盤の買い目も朝の選定で固定する**（朝の選定 → 記録◎ の軸だけ → なし）。盤の `fixed_selection` は組んだ買い目の軸・相手をそのまま出す（取消の扱いを書き直さない）。盤は読むだけ。API の `fixed_selection` で固定した構造と朝の時刻を返し、web の盤に 🔒 で出す。
6. **検証手段**: `gate_calibration.py` に「朝の固定」節を足し、朝の選定があるレースで発走前の全スイープの選定が朝の選定から外れないこと（`0/N`）を REQ-D01-003 の検証手段にする。

#### 理由

- 朝のスナップショットから機械的に組めば、predict を実行しなくても朝の選定ができ、predict-watch の起動が遅れても朝の時点で作れる。ADR 0078 が却下した「記録◎ のみを固定の材料にする」（人手の記録頼み）の弱点を持たない。
- 保存するのは、ADR 0078 が in-memory の固定を却下したのと同じ理由（`--once`・再起動で効かない。統計や出馬表の更新で選定が変わりうる）。
- `live_ev_snapshots` に朝の行を入れると、「最古行＝初回スイープ」を前提にした計測（gate_calibration・#724 の再構成）と盤のフリップ判定を壊す。
- 窓に入った後の入れ替えは軸フリップそのもので、REQ-D01-003 に反する（計画の敵対的レビューで指摘）。

#### 却下した代替案

- **朝の predict / overview の結果を記録する**: 実行しないと朝の選定が無い。「朝」の時刻も実行時点になり、overview は read-through で取り直したオッズで組むので #724 の朝ともずれる。
- **`live_ev_snapshots` に朝の行を入れる（マイグレーション不要）**: 上の理由。ADR 0078 の「マイグレーション不要」の前提はここで外す。
- **保存せず毎回計算する**: ADR 0078 の in-memory の固定の却下と同じ理由。
- **記録◎ を軸だけ後から差し替える**: 朝の軸と食い違うと相手が 4 頭に減り、モデルの首位が買い目から消える。朝の計算で軸に強制する。
- **切り替えのフラグを作る**: 要件を満たすための既定の変更で、戻す運用の需要が無い（YAGNI）。

#### 影響

- ADR 0078 からの変更: 固定の優先順に朝の選定が入る。「マイグレーション不要」の前提を外す（`morning_selections` を足す）。ADR 0078 が対象外にした「盤面の相手も同時に固定する」を、朝の選定という基準ができたので採る。#601 の「固定時点を朝へ前倒しする」の却下（1 日 3R が根拠）は、#724 で根拠が規範に置き換わったので上書きする。
- 運用: マージ後の最初のビルドの直後に `paddock-analyze migrate` を流す（keiba-start Step 0.2 のとおり）。ビルドしてから migrate するまでの間は、全バイナリが起動を拒む。朝の選定は predict-watch が動いて初めてできるので、盤に朝の固定が出るのも predict-watch の起動後。
- 計測への影響: `paddock-analyze reconstruct-slips` と `fix_timing_probe.py` は「`live_ev_snapshots` の最古行＝固定なしで組んだ T-40 の選定」を前提にしている。#767 以降の日付では最古行が朝の固定で組まれるので、再構成の検査（記録との一致）には使えない。
- 記録◎ を固定に効かせたいときは、predict-watch が朝の選定を作る前（初回のスイープより前）に入れる。保存した後に入れた記録◎ は、盤・predict-watch のどちらでも軸を替えない（#601 では T-40 まで効いていたので、効く期間は短くなる）。記録◎ の経路は `predictions` が空で実地では動いていない（ADR 0078 の留保のまま）。
- 推定の非対称: 朝の選定は `predict_race_views_at`（出走馬はそのスナップショットの単勝に載っている馬・統計は開催日より前・馬場なし）で組み、朝の選定が無く初回スイープにフォールバックしたレースは本番の views（出馬表の全馬・最新オッズ）で組む。フォールバックしたレースだけ入力が違う。
- 既知の制約（ユーザー判断で守りは足さず記録だけにした）: 朝の選定が初回スイープの後に保存・読み出される経路では、固定が一度入れ替わりうる。DB の部分障害で、あるスイープだけ朝の選定を一括でもレース単位でも読めず、初回スイープの保存だけ成功したレースは、次のスイープで朝の選定に切り替わる。`--window` の違う predict-watch を二重に起動したときも同じ形になる。盤は初回スイープを見ないので、このとき盤と predict-watch が食い違う。どちらも gate_calibration の「朝の固定」節で外れとして見える。
- 延期・代替開催で race_id が同じまま日付が変わったときは、前の日の朝の選定を使わない（`morning_selections` の主キーが race_id なので新しい日の選定も保存できず、初回スイープの固定に戻る）。
- 盤の買い目（`/board`）は固定（記録◎・朝の選定）を通すが、`/recommendations` と対話の `predict` は固定しない。固定があるレースでは値が違う（`predict` の記録との関係は別 issue）。

### #776: 対話の predict の買い目を盤と同じ朝の固定で組む (2026-10-25) — 採用

#### コンテキスト

- #767 で predict-watch と盤の買い目（軸・相手・混戦）を朝の選定で固定したが、対話の `paddock-predict` は固定なし
  （`PinnedSelection::default()`）で組んだ買い目を表示し、`y` でそのまま `predict_bets` に記録していた。#767 の影響の最終行で
  「`predict` の記録との関係は別 issue」とした件。
- `predict_bets` に書く経路は 3 つ（web の盤・desktop の盤・CLI の predict）で、固定を通っていないのは CLI の predict だけだった。
  🔒 に従って買うと記録と実際の購入がずれ、REQ-D01-006 の実績集計（`bet_records` の ◎）の母集団に「固定なしの実行時点の首位」と
  「朝の固定」の 2 つの定義が混ざっていた。
- 朝の選定（`morning_selections`）を書くのは predict-watch だけで、keiba-start は predict（Step 2）を predict-watch の起動（Step 5）より
  先に回す順番だった。

#### 決定

PO の回答（質問票・`qa/QA-predict-fixed-selection-776.md`）に従う。

1. 盤の固定（同じ日の朝の選定 → 記録◎ の軸だけ → なし。初回スイープは使わない）を use-case の `board_fixed_selection` に切り出し、
   盤と対話の predict が同じ関数で組む。predict は `fixed_portfolio` で固定済みの買い目を組み、表示と記録に使う。
2. 対話・`--skip-all`・`--overview` のすべてで固定済みの買い目を出す。各レースの買い目の見出しの直下に、predict-watch のログと同じ形の
   `🔒 軸… 相手… 固定（朝 HH:MM）` を出す。出所が朝の選定でない（記録◎ の軸だけ・固定なし）ときは、相手・混戦が実行時点の
   オッズなので「predict では記録しない」を添える（`session::pin_source_lines`）。
   🔒 行の本体（`predict_format::format_pin_body`）と出所の文言（use-case の `fix_source_label`）は predict-watch と共有する
   （predict-watch の出力は golden テストでバイト単位に固定して変えていない）。
3. `/recommendations` は固定しない（呼び出している画面が web にも desktop にも無い）。
4. keiba-start に Step 1.7 を足し、predict-watch の起動を predict（Step 2）より前に移す（通知の閾値を下げた起動もここで行う）。
   朝の選定ができたかは predict の各レースの出所の行で判定する（predict-watch のログの件数の行はレースを特定できない）。
5. 朝の選定が無いレースが残っていても predict は回してよい。そのレースは predict では記録せず、朝の選定ができてから盤の執行パネルで
   記録する（2026-10-31 追加の PO 判断・qa Q7）。これを運用だけに任せず、出所が朝の選定でないレースの記録の手前に確認を挟む
   （`may_record_unfixed_race`。既定は記録しない、`y` のときだけ記録する）。完全には禁止しない——predict-watch を回していない日は
   朝の選定がずっと作られず、盤から記録しても同じく固定なしになるため（#623 の「確認を経れば通す」と同じ形）。
6. 記録◎（人手の ◎）を固定に効かせるなら、predict-watch の起動（Step 1.7）より前に取り込む。運用はこれを書くだけにする
   （記録◎ の経路は #767 の時点で実地では動いていない。2026-10-31 追加の PO 判断・qa Q6）。

#### 理由

- 記録の 3 経路が同じ関数で組むので、`predict_bets` の買い目の構造の定義が 1 つになる。規則（`resolve_fixed_selection`）を二重に実装しない。
- 朝の選定は「当日 JST 0 時以降で最初の単勝」から組むので、predict-watch を早く起動しても選定の中身は変わらない。起動の順番を
  入れ替えるだけで、predict の時点で朝の選定がある状態にできる。

#### 却下した代替案

- **運用で吸収する**（コードは変えず「🔒 で買うときは盤から記録する」と書く）: CLI の `y` で記録すると母集団が混ざり続ける（PO 判断）。
- **predict の側で `ensure_morning_selection` を呼んで朝の選定を作る**: #767 で「朝の選定は predict-watch が計算・保存する」と決めており、
  書き手を増やすと先勝ちの競合が増える。
- **opt-in のフラグにする**: #767 で切り替えフラグは YAGNI として却下済み。
- **predict でも初回スイープを固定の元に渡す**: 盤と違う規則になり、web・desktop の記録とそろわない。
- **`--overview` / `--skip-all` は実行時点のまま**: 表示の経路が 2 つに分かれる（PO 判断）。

#### 影響

- 朝の選定があるレースでは、predict の表示・記録の軸・相手・混戦が盤と同じになる。朝の選定ができる前（predict-watch の起動前・
  odds-collect の初回スイープ前）に predict を回したレースは、従来どおり固定なし（記録◎ があれば軸だけ）で組まれる。
- 出所が朝の選定でないレースは、確認で `y` を選んだときだけ記録される。その記録は固定なしの構造のまま `predict_bets` に入る。
- `predict_bets` には固定の出所を残さない（盤の 2 経路も同じ。出所の記録は別 issue）。#776 のマージ日より前の CLI の記録は固定なしで、マージ後も朝の選定ができる前に記録した
  レースは固定なし。REQ-D01-006 の集計で母集団を分けるときは、日付と predict-watch のログ（朝の選定の確定時刻）で判断する。
- predict の 🔒 行は固定の値（取消を反映する前）を出す。盤は取消を反映した後の買い目の軸・相手を出すので、取消のあるレースでは
  表示が食い違いうる（買い目そのものは同じ `compose_portfolio` の規則で組まれる）。
- 馬連 vs 馬単の EV 診断は軸ロックの対象外のまま（常に勝率首位）。固定後の predict では、買い目の軸と診断の軸が違って並びうる。
- 朝の選定の読み出しの DB エラーは、`predict_race_views` と同じく伝播する（セッションは止まる。黙って固定なしにはしない）。
- predict-watch を先に起動すると、窓の前の全レースの朝の選定が最初のスイープで保存される。その後に取り込んだ記録◎ は固定に入らない
  （#767 の規則）。改変前の手順（predict-watch の起動が Step 5）より、記録◎ が効く期間は短くなる。
- 朝の選定が作られないレース（窓に入った後の単勝しか無い・その日に predict-watch を回していない過去日〈#767 より前など〉の `--overview`）は、
  predict と盤ではずっと朝の選定の固定にならない。
  predict-watch はこのレースを初回スイープで固定するが、盤と predict は初回スイープを持たないので、記録の経路では predict-watch の 🔒 と
  同じ構造にならない（既知の制約）。
- #767 の影響の最終行（「`/recommendations` と対話の `predict` は固定しない」）のうち、対話の `predict` の部分を本エントリで置き換える。

