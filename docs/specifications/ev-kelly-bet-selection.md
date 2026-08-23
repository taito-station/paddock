---
# knowledge 規約に基づくメタデータ（docs/knowledge/README.md）。specifications はその場で
# knowledge に昇格（ADR 履歴・相互リンクを壊さないため物理移動しない）。
status: Confirmed
kind: knowledge
doc_class: [D23, D22]
tags: [D23, D22]
updated: "2026-08-12"
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

> 横断の用語索引は [用語集](../knowledge/glossary.md)（D07）。定義の正本は本書で、用語集はここを指す。

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

> **置き場所**: 本書は D23 の主 spec（[doc-classes.md](../knowledge/doc-classes.md) の割当）なのでここに置く。
> 棄却の詳細と実測表は [betting-rule-history.md](betting-rule-history.md)（D24 主）にあり、本表は**合否バーの
> 数値だけを再掲**する（バーを本文から辿らせると検証手段として成立しないため）。
>
> **現行ルールの運用指示はリポジトリルートの `CLAUDE.md`「買い方ルール」節が正**（毎セッション自動で
> 読まれることが実効性の source。[doc-classes.md](../knowledge/doc-classes.md) の「体系側の既知の穴」参照）。
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
| REQ-D23-007 | `predict-watch` の買い目選定（軸・相手・混戦判定）は**当日の初回スイープで確定し、以後オッズで動かさない**。固定した相手が取消なら落とすが**ライブ順位で補充しない**（点数が減る）。固定の優先順は 記録◎ → その日の初回スイープ → 固定なし | `cargo test -p paddock-domain` の `pinned_selection_survives_market_movement_while_roi_moves`（オッズを差し替えた 2 スイープで選定が一致し ROI は動く／固定しなければ動く）・`forced_partners_drops_scratched_without_backfill`、`cargo test -p rdb-gateway --test test_live_ev_persistence` の `pins_return_the_earliest_sweep_not_the_latest`（**最古**を返すこと）、および開催日に `scripts/predict-check/gate_calibration.py` の「軸（◎）の安定性」節が `0/N` になること | ADR 0078 | Confirmed |
<!-- REQ:end D23 -->

**D01 と重複させない。** 「ROI ≥ 100% のレースだけ張る」「軸ロック＋ズレ増額」「買い目の提示形式と
配分の正」は [product-goals.md](../knowledge/product-goals.md) の
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
[601-axis-flip-in-predict-watch.md](601-axis-flip-in-predict-watch.md)。

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
# → docs/original-docs/601-axis-flip-in-predict-watch.md「再現方法」の SQL

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
