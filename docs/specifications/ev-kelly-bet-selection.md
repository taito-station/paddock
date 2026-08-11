---
# knowledge 規約に基づくメタデータ（docs/knowledge/README.md）。specifications はその場で
# knowledge に昇格（ADR 履歴・相互リンクを壊さないため物理移動しない）。
status: Confirmed
kind: knowledge
doc_class: [D23, D22]
tags: [D23, D22]
sources:
  - docs/original-docs/0003-ev-kelly-bet-selection.md
  - docs/original-docs/0019-portfolio-generator.md
  - docs/original-docs/0028-konsen-odds-trigger-rejected.md
  - docs/original-docs/0030-konsen-trio-partner-width-rejected.md
  - docs/original-docs/0033-conditional-win-bet-rejected.md
  - docs/original-docs/0041-umaren-only-strategy-rejected.md
  - docs/original-docs/0043-exacta-in-portfolio-rejected.md
  - docs/original-docs/0046-allocation-prob-weight-no-floor-rejected.md
  - docs/original-docs/0054-kelly-staking-rejected.md
  - docs/original-docs/0064-live-ev-buy-view.md
  - docs/original-docs/0065-wide-partners-top5-alignment.md
distilled_from_sha: "faa62d6"
updated: "2026-08-11"
---

# 期待値計算・買い目選択・Kelly 配分ロジック仕様書

Issue #12 対応。推定確率とオッズから期待値（EV）を計算し、馬連重視で買い目を選択、
Kelly 基準で賭け額を決定する。

> **用途の限定（#407・2026-07）**: 本仕様の `select_bets` ＋ Kelly 配分は **本番の買い目配分では使われていない**。
> 本番 `predict` の配分は `build_portfolio`（ワイド・馬連・三連複の◎軸ながし、券種予算を 100 円単位で均等配分。
> [ADR 0019](../original-docs/0019-portfolio-generator.md)。券種内の均等配分は [ADR 0046](../original-docs/0046-allocation-prob-weight-no-floor-rejected.md) で
> 確率重み化を棄却し維持）で、Kelly 配分は 71R walk-forward で回収率が現行に劣後し **棄却済み**
> （[ADR 0054](../original-docs/0054-kelly-staking-rejected.md)）。**`select_bets` は現在 backtest 評価（`analyze backtest`）でのみ現用**、
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
[docs/knowledge/README.md](../knowledge/README.md)）。

> **現行ルールの運用指示はリポジトリルートの `CLAUDE.md`「買い方ルール」節が正**（毎セッション自動で
> 読まれることが実効性の source。[doc-classes.md](../knowledge/doc-classes.md) の「体系側の既知の穴」参照）。
> ここはその**根拠と検証手段**を REQ として固定する。棄却の詳細は
> [betting-rule-history.md](betting-rule-history.md)。

<!-- REQ:begin D23 -->
| REQ-ID | 要件 | 検証手段 | 出典 | status |
|---|---|---|---|---|
| REQ-D23-001 | 券種は 3 つ（ワイド / 馬連 / 3 連複）に固定する。単勝・馬単・馬連特化は追加しない | 各棄却 ADR のバックテストを再実行し、**その ADR が置いた baseline** を上回らないこと（0033 = 3 券種 79.5%・**α=0.3 時代**なので production 構成で測り直す / 0041 = **無フィルタ対照**の馬連 ◎軸 top5 78.7%〜79.0%（ゲート込みの `baseline_pf` は 13 鞍 24.5% と小標本で、馬連特化の一部 variant がこれを上回る——**バーに使わない**。0041 が退けた根拠は無フィルタ対照との比較と「>100% セルは 6〜14 鞍・的中 1 鞍のアーティファクト」という留保）/ 0043 = 馬連 top5 単独 78.7%。いずれも 71R）| [ADR 0033](../original-docs/0033-conditional-win-bet-rejected.md) / [0041](../original-docs/0041-umaren-only-strategy-rejected.md) / [0043](../original-docs/0043-exacta-in-portfolio-rejected.md) | Confirmed |
| REQ-D23-002 | 相手は 3 券種とも model top5。広げない | 既定の「相手 5 頭」は ADR 0019 が置いた設計値。**top5 超へ広げないこと**を直接測ったのは **3 連複だけ**（ADR 0030 の相手幅スイープ＝top5 / top7 / 全頭・混戦 90R で回収率が下がる）。**ワイド**の ADR 0065 が測ったのは top3 vs top5 の**絞る側**で（262R・12 開催日で有意差なし＝実装の top5 に寄せた）、上限側は 0030 を援用している。**馬連**は weight-matched な単独スイープ未実施（0019 の既定のまま）。予算非対称の参考対照は ADR 0041 にある（top5 無フィルタ 78.7〜79.0% vs 全頭 flat 66.4%・同 71R）。測り直すなら `scripts/predict-check/strategy_eval.py --partners`。3 連複の相手幅自体は `strategy_eval.py --partners` で掃引できるが、**ADR 0030 と同条件（混戦判定＋印馬ボックス併用）での再走には再実装が要る**——0030 の検証ハーネスは「恒久コードとして残さない」使い捨てで、`strategy_eval.py` に混戦レイヤーが無い | [ADR 0019](../original-docs/0019-portfolio-generator.md)（既定 5 頭）/ [0041](../original-docs/0041-umaren-only-strategy-rejected.md)（馬連の参考対照）/ [0030](../original-docs/0030-konsen-trio-partner-width-rejected.md)（3 連複）/ [0065](../original-docs/0065-wide-partners-top5-alignment.md)（ワイド） | Confirmed |
| REQ-D23-003 | 各点の金額は券種予算内の 100 円単位**均等配分**（`build_portfolio` / `distribute`）。確率重み化と脚ごと最低 ¥100 の撤廃は採らない | ADR 0046 の 71R 実 ROI 比較を再実行（薄い脚への少額 spread を外すと悪化すること） | [ADR 0046](../original-docs/0046-allocation-prob-weight-no-floor-rejected.md) | Confirmed |
| REQ-D23-004 | fractional Kelly は賭け額配分に使わない。`betting/kelly.rs` は EV 候補選抜（`min_kelly` の curation）に留める | ADR 0054 の同一土俵比較（定額 vs Kelly・71R walk-forward）を再実行し、**定額土俵で Kelly 重みが現行の確率重み配分（ROI 75.5%・σ 92.5）を上回らないこと**、および **bankroll 土俵で full Kelly が破産すること** | [ADR 0054](../original-docs/0054-kelly-staking-rejected.md) | Confirmed |
| REQ-D23-005 | 混戦判定は「◎の model 勝率の 0.70 倍以上が ◎含め 4 頭以上」。**オッズ条件を併用しない** | ADR 0028 のオッズ閾値スイープを再実行（baseline を上回る閾値が無いこと） | [ADR 0028](../original-docs/0028-konsen-odds-trigger-rejected.md) | Confirmed |
| REQ-D23-006 | `scripts/predict-check/` の Python（`live_ev.py`）を**張る買い目の配分に使わない**。オフライン EV レポート専用（配分方式の正が `build_portfolio` であることは REQ-D01-007。ここはその裏返しの禁止事項） | `build_portfolio` の単体テストと、`predict` / `predict-watch` が同一関数を通ること | [ADR 0064 の追補（#346）](../original-docs/0064-live-ev-buy-view.md)——**0064 本体の決定は逆**（当時はライブ writer を Python `live_ev.py` に一本化するとしていた）。Rust に一本化したのは追補側 | Confirmed |
<!-- REQ:end D23 -->

**D01 と重複させない。** 「ROI ≥ 100% のレースだけ張る」「軸ロック＋ズレ増額」「買い目の提示形式と
配分の正」は [product-goals.md](../knowledge/product-goals.md) の
**REQ-D01-001 / 003 / 007** が正本で、ここでは採番し直さない（同じ要件に ID が 2 つあると、片方だけ更新されても機械検査は検出できない——
ADR 0073 / 0074 が排除したい二重管理そのものになる）。本表はその下で**買い方の具体**を決める要件に限る。

> D23 の採番は #594 が初出で、重複解消で落とした分は**公開前に詰め直した**（欠番は作っていない）。
> 一度公開した番号は再利用しない——廃止するときは行を残して `status: Retired` にする。

