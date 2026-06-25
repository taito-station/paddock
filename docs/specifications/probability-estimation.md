# 着順確率推定モデル仕様書

Issue #11 対応。DB に蓄積された過去成績をもとに、出走馬ごとの 1 着・2 着・3 着確率を推定する。

## 概要

![着順確率推定フロー](diagrams/probability-estimation-flow.svg)

出馬表（`RaceCard`）を受け取り、各馬の勝率（win）・連対率（place）・複勝率（show）の推定確率を返す。
精密な機械学習モデルではなく、「データがあれば動く」ルールベーススコアリングを採用する。

---

## 用語定義

本文書での英語命名は日本語競馬用語に対応させている（国際標準と異なる場合がある）。

| フィールド名 | 日本語 | 定義 |
|------------|-------|-----|
| `win_prob` | 勝率 | 1 着以内確率 |
| `place_prob` | 連対率 | 2 着以内確率（日本競馬の「連対」＝top-2） |
| `show_prob` | 複勝率 | 3 着以内確率（日本競馬の「複勝」＝top-3） |

---

## 入力

| 項目 | 型 | 説明 |
|------|----|----|
| `RaceCard` | ドメイン型 | race_id / venue / distance / surface / entries |
| `HorseEntry` (entries 内) | ドメイン型 | gate_num（枠番）/ horse_num / horse_name / jockey (Option) |

> `gate_num` はスコアリング時に `course_stats` の枠順グループ（Inner/Middle/Outer）に変換して `course_gate_rate` を引くために使用する。

## 出力

| 項目 | 型 | 説明 |
|------|----|----|
| `horse_num` | `HorseNum` | 馬番 |
| `horse_name` | `HorseName` | 馬名 |
| `win_prob` | `f64` | 勝率・1 着確率（0.0〜1.0） |
| `place_prob` | `f64` | 連対率・2 着以内確率（0.0〜1.0） |
| `show_prob` | `f64` | 複勝率・3 着以内確率（0.0〜1.0） |

勝率は 1 着＝1 ポジションなのでレース内合計 1.0、連対率は 2 着以内＝2 ポジションなので合計 2.0、
複勝率は 3 着以内＝3 ポジションなので合計 3.0 へ正規化する（各馬は確率として 1.0 で上限クランプ）。
さらに馬ごとに累積 max で単調化し `win_prob ≤ place_prob ≤ show_prob` を保証する（ADR 0007）。

---

## スコアリングアルゴリズム

### ステップ 1: 統計データの取得

各 `HorseEntry` に対して以下 3 種のスタッツをDBから並列取得する。

| スタッツ | キー | スコアリングに使用するデータ |
|---------|-----|----------|
| `course_stats` | venue × distance × surface | **枠順グループ別** 勝率・連対率・複勝率（`course_gate: Option<RateTriple>`, 実績なしは None=母数除外, #81） |
| `horse_stats` | horse_name | **芝ダ別** 勝率・連対率・複勝率（`horse_surface: Option<RateTriple>`, #81）・**距離帯別** 勝率・連対率・複勝率（`horse_distance: Option<RateTriple>`, #81）・**馬場状態別** 勝率・連対率・複勝率（`horse_track_condition: Option<RateTriple>`, #73） |
| `jockey_stats` | jockey_name (任意) | **芝ダ別** 勝率・連対率・複勝率（`jockey_surface: Option<RateTriple>`, 騎手なし／実績なしは None, #81） |
| `trainer_stats` | trainer_name (任意) | **芝ダ別** 勝率・連対率・複勝率（`trainer_surface: Option<RateTriple>`, #74）。母数は `results.trainer` |
| `find_recent_runs` | horse_name × cutoff 日 | **直近 N 走（N=1〜3）** から算出する前走フォームトレンド（`recent_form: Option<f64>`, #31/#220）。重みは [1.0, 0.5, 0.25] 固定の加重平均。cutoff より前のみ（リーク防止） |
| `standard_times` | cutoff 日（レース共通, 1回） | (surface,distance) 別のコーパス標準タイム[秒]（前走タイム相対速度の分母, #76）。`date < cutoff` で as-of 集計、薄いバケツは除外 |

### ステップ 2: 生スコア計算（重み付き平均）

馬ごとに、**存在する factor のみ**の重み付き平均を計算する（勝率・連対率・複勝率それぞれ）。

```
raw_score =
    ( [ 2.0  × course_gate_rate ]    // コース×枠順（最も信頼度高, 実績ありのときのみ, #81）
    [ + 1.0  × horse_surface_rate ]  // 馬の芝ダ実績（実績ありのときのみ, #81）
    [ + 1.0  × horse_distance_rate ] // 馬の距離帯実績（実績ありのときのみ, #81）
    [ + 1.0  × jockey_surface_rate ] // 騎手の芝ダ実績（騎手あり×実績ありのときのみ, #81）
    [ + 1.0  × trainer_surface_rate ] // 調教師の芝ダ実績（調教師あり×実績ありのときのみ, #74）
    [ + 1.0  × horse_track_condition_rate ] // 馬の馬場状態別実績（馬場状態あり×実績ありのときのみ, #73）
    [ + 0.25 × recent_form ]         // 前走フォームトレンド[0,1]（前走ありのときのみ, #31/#220）
    [ + 0.25 × weight_carried ]      // 斤量のレース内相対[0,1]（斤量あり×field平均ありのときのみ, #135）
    ) / Σ(present weights)           // 例: 全項あり=7.5、全項なし: Σ重み=0（下記 注2）
```

> 注1: **「実績なし」の項はその項と重みを母数から除外する（0 埋め＝全敗扱いにしない）**（ADR 0007/0014）。
> 当該グループの出走実績が無い（グループ不在・出走 0 件）factor は全て `None`: コース枠順
> （`course_gate`）・馬の芝ダ／距離帯（`horse_surface`/`horse_distance`）・騎手（`jockey_surface`、騎手
> 未登録も含む）・調教師（`trainer_surface`）・馬場状態（`horse_track_condition`）・前走（`recent_form`）・
> 斤量（`weight_carried`、斤量未取得 or field 平均なし）。
> #81 で 0 埋めだった course_gate/horse_surface/horse_distance/jockey_surface を None 除外へ統一した。
> これらの項は「平均からの差分」としてのみ効き、欠落で不当に減点されない。全馬が同条件のときは定数除算
> となり相対順位は不変。
> 注2: 全 factor が `None`（どの統計も実績なし）の馬は `weight == 0` → ゼロ除算を避けて score = 0.0。
> score 0 の馬は次ステップの均等フォールバックに畳まれる（ADR 0014）。
> 注3: `recent_form` は直近 N 走（N=1〜3、backtest #220 評価の結果 N=1 を維持）の各走スコアを
> 重み [1.0, 0.5, 0.25] で加重平均したトレンドスカラー値（0.5=中立）。各走スコアは馬体重変化・
> 前走人気乖離・前走間隔・前走着差（#76）・前走タイム相対速度（#76）を [0,1] に統合した値。
> スコアが取れない走（中止・情報欠落等）は分母から除外する（欠落フォールバック）。
> N=1 のとき前走 1 走のみ（#31 の現行挙動）と完全一致する。
> 全ての前走でスコアが取れない場合は `None`（`recent_form` 項を除外）。
> win/place/show に同値で寄与し、factor 重み 0.25 は ADR 0009 で決定。
> 前走着差は圧勝ほど高く・大敗ほど低い競争力シグナルで、着差文字列が解釈不能・着順なしの前走では sub-signal を落とす。
> 前走タイムは (surface,distance) 別のコーパス標準タイム（as-of 集計, `standard_times`）に対する相対速度で、
> 標準より速い前走を加点・遅い前走を減点する。前走タイム無し・標準タイム未整備の前走では sub-signal を落とす。
> 注4: `horse_track_condition` は評価対象レースの馬場状態（良/稍重/重/不良）に対応する馬の成績。
> レースの馬場状態が未確定（backtest: DB に無い / predict: `--track-condition` 未指定）のとき、または
> その馬場での出走実績が無い（グループ不在・出走 0 件）馬は `None`。重み 1.0 はバックテストで決定
> （ADR 0011）。出馬表 PDF に馬場状態は無いため、predict 経路では呼び出し側が当日の値を渡す
> （予想セッションはレース毎の対話入力＝DB 値があれば空入力でデフォルト採用、analyze CLI は
> `--track-condition`）。
> 注5: `trainer_surface` は調教師の芝ダ別成績（#74）。調教師名は出馬表 `HorseEntry` に無いため、
> predict は **netkeiba 出馬表**から取った `entry.trainer`、backtest は `results.trainer`（当該レース
> 確定値）を使う。調教師なし／該当 surface 実績なしの馬は `None`。重み 1.0 は jockey と同値（ADR 0012）。
> **現状の制約**: 統計母数 `results.trainer`（および netkeiba 過去走）が未充足のため、本項は実データ上
> まだ発火しない。母数充足（結果 PDF / netkeiba 過去走の trainer 抽出）は別 Issue。出馬表 PDF パーサの
> trainer 抽出も別 Issue（PDF 経路は当面 `trainer=None`）。

### ステップ 2.5: ベイズ縮約（#75, ADR 0016）

各 factor のレートを母集団 prior（出走頭数 ~14 由来の基準率: win=1/14, place=2/14, show=3/14）へ
出走数 `k` に応じて縮約する:

```
smoothed = (k · rate + m · prior) / (k + m)        // m = 擬似カウント
```

`k≫m` で生レート、`k=0` で prior、その間を単調に補間する。少データ馬（新馬・復帰馬）の極端な
レート＝過信（`win_prob=0` を含む, ADR 0002）を緩和する。`m` は `EstimationConfig.shrinkage` で
切り替え可能で、backtest（2026-03-28〜05-31 / 144R）で m=10 が単勝 Brier/LogLoss・連対で最良
（単勝的中 9.7→13.2%）だったため**本番 predict は m=10 を既定**とする（ADR 0016）。

> **リーセンシー重み付け（recency, #75 Phase B）** も `EstimationConfig.recency` /
> `--recency-half-life` で切り替え可能（馬の芝ダ・距離帯・馬場状態を `0.5^(days_ago/half_life)` で
> 時間減衰集計）。ただし backtest で改善が確認できず（前走フォーム #31 が直近を既に捕捉・カテゴリ別
> 出走数が疎）、**デフォルトは無効**。機構は将来評価のため残す（ADR 0016）。

### ステップ 3: レース内正規化（top-k）+ 単調化

各列を「着以内ポジション数」に対応する合計へ正規化し、各馬を確率として 1.0 で上限クランプする。

```
win_prob_i   = min(1, raw_win_score_i   / Σ(raw_win_score_j)   × 1.0)   // 1 着 = 1 ポジション
place_prob_i = min(1, raw_place_score_i / Σ(raw_place_score_j) × 2.0)   // 2 着以内 = 2 ポジション
show_prob_i  = min(1, raw_show_score_i  / Σ(raw_show_score_j)  × 3.0)   // 3 着以内 = 3 ポジション
```

その後、馬ごとに累積 max で単調化して `win_prob ≤ place_prob ≤ show_prob` を保証する。

```
place_prob_i = min(1, max(win_prob_i,   place_prob_i))
show_prob_i  = min(1, max(place_prob_i, show_prob_i))
```

> win 列は各馬のシェア ≤ 1 のため上限クランプは発生せず合計は厳密に 1.0。place/show は小頭数
> （n < 3）で上限クランプにより合計が 2.0 / 3.0 を下回りうる（確率の上限を優先）。
> 例: 3 頭立ては全馬が複勝圏なので show_prob = 1.0。

**フォールバック条件:**
- 個別馬のスコアが 0（スタッツ未蓄積等）の場合: その馬のスコアは 0.0 のまま正規化に含める（その馬の win_prob は 0.0。単調化により place/show も 0.0 のまま）
- **全出走馬のスコア合計が 0**（出走馬全員のスタッツが未蓄積）の場合のみ均等フォールバック:
  `win_prob = 1/n`、`place_prob = min(1, 2/n)`、`show_prob = min(1, 3/n)`（自然に単調）

---

### ステップ 4: 市場オッズ（単勝）ブレンド（#72, 任意）

モデルは過去成績ベースで、スタッツ希薄馬（新馬・復帰馬）に弱い。市場の単勝オッズは多くの予想家の
集合知が集約された強力かつ高校正な信号なので、モデルの win 確率と線形ブレンドして補正する
（`blend_with_market_win`）。**ブレンド係数 α = モデル重み**（`1-α` が市場重み, 範囲 [0,1]）。

```
implied_i      = 1 / odds_i                          // 単勝オッズ → implied 確率
market_prob_i  = implied_i / Σ implied_j             // 控除率(オーバーラウンド)を除去し合計 1.0
blended_win_i  = α · win_prob_i + (1-α) · market_prob_i   // オッズの無い馬はモデル値のまま
win_prob_i     = blended_win_i / Σ blended_win_j     // 合計 1.0 へ再正規化
place/show_i   = 累積 max で win ≤ place ≤ show を再是正（v1 は win のみブレンド）
```

- `α = 1.0`（既定の CLI 未指定）または市場オッズ空のときはモデルのみ（no-op）。
- 市場オッズの取得元:
  - **predict（本番・未来レース）**: 当日の `race_odds` 最新スナップショット（`as_of = None`）。
  - **backtest**: 当時の `race_odds`（`as_of = レース日`, リーク防止）を優先し、無ければ PDF 確定成績の
    単勝 `results.odds`（クローズ前後の確定オッズ。結果はリークしない）で代替する。過去レースは
    `race_odds` スナップショットが無いことが多いため、この代替で評価可能になる。
- **採用 α（本番既定）= 0.2**（ADR 0034 で 4891R 計測後に確定）。
  初期 backtest（2026-03-01〜05-31, 144R, 市場 = `results.odds`）では α=0.3 が最良だったが、
  拡張バックテスト（2025-01-05〜2026-06-14, 4891R）で α が Brier/LogLoss に単調に効くことを
  確認し 0.2 に更新。詳細は ADR 0034。初期スイープ参考値:

  | α (モデル重み) | 単勝的中 | 複勝的中 | 想定回収率 | Brier(win) | LogLoss(win) |
  |---|---|---|---|---|---|
  | 1.0（モデルのみ） | 12.5% | 31.9% | 67.7% | 0.0672 | 0.6212 |
  | 0.7 | 22.9% | 49.3% | 74.2% | 0.0590 | 0.2187 |
  | 0.5 | 28.5% | 58.3% | 85.1% | 0.0553 | 0.2029 |
  | 0.3 | 34.7% | 66.0% | 91.5% | 0.0529 | 0.1925 |
  | 0.0（市場のみ） | 32.6% | 66.0% | 78.7% | 0.0518 | 0.1841 |

  初期スイープ（144R）では α=0.3 が最良。ただし確定オッズ使用の楽観値。
  α=0.2 はこの初期スイープの対象外（範囲 0.0〜1.0 の粗い刻みで探索）であり、
  拡張バックテスト（ADR 0034）で独立に計測して採用を確定した。
  拡張バックテスト（4891R / ADR 0034）で α が Brier/LogLoss に単調に効くことを確認し **現行採用値 α=0.2 に更新**（α=0.2 は 144R 初期スイープ対象外）。

  > 注 1: このスイープは市場に**確定（クローズ）オッズ**を使うため、live 予想（事前スナップショット
  > `race_odds` を使用）より楽観的な上限値。拡張バックテストでは live オッズで α を再評価し α=0.2 に
  > 確定した（ADR 0034）。
  > 注 2: backtest でブレンド有効時、確率の事前分布に使う市場オッズと回収率評価に使う払戻オッズが
  > 同一ソース（`race_odds`→`results.odds`）のため、回収率は構造的に楽観側へ寄る。回収率は相対比較の
  > 参考値として読む（指標の本命は的中率・Brier/LogLoss）。
  > 注 3: 市場オッズが無いレースは自動でモデルのみにフォールバックするため、ブレンド有効化の副作用は無い。

CLI: `analyze backtest --blend-alpha <α>` / `analyze predict --blend-alpha <α>`（未指定でモデルのみ）。

---

## 統計データ拡張: GroupStat への `shows` 追加

現行の `GroupStat`（`src/use-case/src/repository.rs` で定義）は連対（1〜2 着）までしか保持しない。複勝率（1〜3 着）を扱うため `shows` フィールドを追加する。

```rust
// src/use-case/src/repository.rs
pub struct GroupStat {
    pub label: String,
    pub starts: u32,
    pub wins: u32,
    pub places: u32,  // 連対 (top-2)
    pub shows: u32,   // 複勝 (top-3) ← 追加
}
```

DBクエリは以下を追加:
```sql
SUM(CASE WHEN finishing_position IN (1,2,3) THEN 1 ELSE 0 END) AS shows
```

**影響範囲（全件変更が必要）:**
- `src/interface/rdb-gateway/src/repositories/horse_stats.rs`: 6 クエリパターン（overall / by_surface / by_distance_band / by_track_condition / by_popularity_band / by_gate）
- `src/interface/rdb-gateway/src/repositories/course_stats.rs`: 1 クエリパターン（by_gate_group）
- `src/interface/rdb-gateway/src/repositories/jockey_stats.rs`: 3 クエリパターン（overall / by_surface / by_gate）

---

## レイヤー別実装方針

### Domain (`paddock_domain::prediction`)

```rust
pub struct HorseProbability {
    pub horse_num: HorseNum,
    pub horse_name: HorseName,
    pub win_prob: f64,
    pub place_prob: f64,
    pub show_prob: f64,
}

pub fn estimate_probabilities(
    entries: &[(HorseEntry, HorseFactors)],
) -> Vec<HorseProbability>
```

`HorseFactors` は horse_stats / course_stats / jockey_stats から抽出した率を束ねる中間型。
win / place / show 各率を `RateTriple` で保持する。

```rust
pub struct RateTriple {
    pub win: f64,
    pub place: f64,
    pub show: f64,
}

pub struct HorseFactors {
    pub course_gate: Option<RateTriple>,    // course_stats の枠順グループ率（実績なし時 None, #81）
    pub horse_surface: Option<RateTriple>,  // horse_stats の芝ダ率（実績なし時 None, #81）
    pub horse_distance: Option<RateTriple>, // horse_stats の距離帯率（実績なし時 None, #81）
    pub jockey_surface: Option<RateTriple>, // jockey_stats の芝ダ率（騎手なし/実績なし時 None, #81）
    pub trainer_surface: Option<RateTriple>, // trainer_stats の芝ダ率（調教師なし/実績なし時 None, #74）
    pub horse_track_condition: Option<RateTriple>, // horse_stats の馬場状態別率（#73）
    pub recent_form: Option<f64>,        // 前走フォーム [0,1]（前走なし時 None, #31）
}
```

スコアリングと正規化の純粋関数として実装（IO なし・テスト容易）。

### Use-Case (`use_case::interactor::race::predict`)

```rust
pub async fn predict_race(&self, race_id: &RaceId) -> Result<Vec<HorseProbability>>
```

1. `find_race_card(race_id)` → RaceCard 取得
2. 各 HorseEntry に対してスタッツを **逐次取得**（デフォルト）。性能要件が出た場合に `join_all` 等で並列化可
3. `domain::prediction::estimate_probabilities` を呼ぶ

Repository に `find_race_card` メソッドを追加する。

### Interface (rdb-gateway)

- `find_race_card` SQL: race_cards / race_card_entries テーブルから取得
- 既存の horse_stats / course_stats / jockey_stats クエリに `shows` カラムを追加

### Apps (analyze)

```
paddock-analyze predict <race_id>
```

出力例：
```
# レース予測 2026060412R02
馬番  馬名            勝率     連対率   複勝率
  1  ガリレオトライ  18.3%   36.7%   55.1%
  2  テスラブルー    12.1%   24.2%   36.3%
  ...
```

---

## 既知の制約

- スタッツの**ある**馬で当該グループのみ出走 0 件の factor は `None`（母数除外）になり、0 埋め減点しない
  （#81/ADR 0014）。全 factor が `None`（どの統計も実績なし）の馬は score = 0。レース内の他馬に正スコアが
  あれば正規化で確率はほぼ 0（限りなく低い）になり、**均等フォールバックはレース全馬が score 0 のときのみ**
  発動する。
- コースデータが存在しない組み合わせ（venue × distance × surface）の場合、`course_gate = None`（母数除外）
  として計算する（#81 以前は 0 埋め）
- モデルは過去成績・前走フォーム（馬体重変化・前走人気乖離・前走間隔, #31／前走着差・前走タイム相対速度, #76）・
  馬場状態別成績（#73）・斤量のレース内相対（#135）を使用。
- 斤量項（`weight_carried`）は当該レースの field 平均斤量との kg 差を [0,1]（0.5=中立）に写像する独立 factor。
  向きは「平均より重い→加点」で、backtest（両符号比較）で減点符号より的中率・回収率・校正がすべて良かった
  ため採用（実績馬ほど重い斤量を課される選択効果, ADR 0009 追補）。斤量は netkeiba 出馬表のみ取得（PDF 出馬表・
  斤量欠落・field 平均なしは項なし）。backtest は results の確定斤量で field 平均を取る。
- 馬場状態項は評価対象レースの馬場状態が分かるときのみ効く。出馬表 PDF に馬場状態は無いため、
  analyze CLI の predict では `--track-condition` の手入力が必要（未指定なら項なし＝従来どおり）。
  重み 1.0 はバックテストで「単勝/連対/複勝/回収率 改善・単勝 Brier/LogLoss 微悪化」を確認した
  ピーク値（ADR 0011）
- 前走フォームは前走（直近 1 走）が DB に無い馬では `None`（寄与なし）。取り込み済み成績が乏しいデータ
  セットでは効果が限定的。重み 0.25 はバックテストで「連対/複勝/回収率/Brier 改善・LogLoss 微悪化」を
  確認した保守値（ADR 0009）
- `win_prob ≤ place_prob ≤ show_prob` の単調性は **保証される**（top-k 正規化 + 累積 max 単調化, ADR 0007）。
  place/show は 2/3 着以内の実確率として扱える（複勝 EV もこの値を使用）
- 小頭数では上限クランプにより place/show の合計が 2.0 / 3.0 を下回る（確率の上限を優先）
- 全馬スタッツ皆無の均等フォールバック時は place/show が高め（小頭数では show=1.0）に出るため、
  複勝 EV（`show_prob` 使用）がオッズ次第で買い目を誘発しうる。情報ゼロ時の買い目抑制は将来課題
- 確率の絶対値より**レース内の相対的な傾向**を見るための参考値として使うこと
