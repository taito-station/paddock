---
status: Confirmed
kind: knowledge
sources:
  - docs/qa/QA-factors-explanation-unify-409.md
  - docs/adr/0014-none-baseline-exclusion.md
  - docs/adr/0055-ev-layer-separation-circular-break.md
distilled_from_sha: "5886f61"
updated: "2026-07-21"
---

# scoring 経路の factor 収集構造

`predict.rs` における条件別成績（factor）の収集・共有の確定知。#409 の qa を蒸留したもの。
決定の経緯・根拠は frontmatter `sources` を参照。

## 全体構造

`collect_race_factors`（`predict.rs:127`）のループ内で、馬ごとに以下の順序で factor を組み立てる:

1. `resolve_shared_factors(...)` を **1 回だけ** 呼び `SharedFactorStats` を構築
2. `build_factors(&shared, ...)` → `HorseFactors`（score 計算の入力）
3. `build_explanation(&shared, ...)` → `HorseExplanation`（UI 表示用の根拠）

**同一の `SharedFactorStats` を両関数に渡す**ことで、ラベル選択と `stat_to_triple_opt` の評価が 1 回に統一されている。

## SharedFactorStats（`predict.rs:427-448`）

`build_factors`（score）と `build_explanation`（根拠表示）が共有する中間構造体（#409 で新設）。

### フィールド構成

```
解決済みラベル（4 本）:
  surf_label     : &'static str  -- 芝ダ区分文字列
  dist_label     : &'static str  -- 距離帯文字列
  gate_label     : &'static str  -- 枠順グループ文字列
  venue_label    : &'static str  -- 競馬場の日本語場名（#350 相性 factor の照合キー）

集計 FactorStat（10 スロット）:
  course_gate            -- コース×枠（全馬共通）
  horse_surface          -- 馬の芝ダ成績
  horse_distance         -- 馬の距離帯成績
  horse_track_condition  -- 馬の馬場状態別成績（馬場未確定・実績なし → None）
  jockey_surface         -- 騎手の芝ダ成績（騎手未登録 → None）
  trainer_surface        -- 調教師の芝ダ成績（調教師未登録 → None）
  jockey_venue           -- 騎手の競馬場別成績（#350）
  jockey_distance        -- 騎手の距離帯成績（#350）
  jockey_horse_combo     -- 馬×騎手コンビ成績（horse.by_jockey を現騎手名で引く）
  horse_venue            -- 馬の競馬場別成績（#350）
```

母数 0・欠落・欠員はすべて `None`（ADR 0014 の None 母数除外）。jockey/trainer 未登録は outer `None`、実績なしは inner `None` の二段で畳む。

## resolve_shared_factors（`predict.rs:459-503`）

ラベル選択と `stat_to_triple_opt` の呼び出しを一か所に集約した純粋変換関数。

- **呼び出しタイミング**: `collect_race_factors` の馬ごとのループで 1 回だけ呼ぶ
- **本番 predict・backtest 両方から共有する**（ADR 0014 の predict/backtest で同一 factor 評価の保証）
- `as_of=None`（全期間統計）は predict 経路。backtest は as-of 統計を渡す

## build_factors（`predict.rs:505-568`）

`SharedFactorStats` の 10 スロットを読み `HorseFactors` を返す純粋変換。

**recency 有効時の乖離点**:
- `horse_surface` / `horse_distance` / `horse_track_condition` の **馬系 3 因子のみ**、recency 有効時（`config.recency = Some(rc)` かつ `recency` が渡された）は共有の集計レートではなく時間減衰レートで上書きする
- course / jockey / trainer・相性 factor は常に `shared` の集計レートを使う
- **本番は `production()` が `recency: None`** のため上書きは発生しない。recency による score と根拠の乖離はこの関数の 1 か所に閉じ込められている（#409 解消）

## build_explanation（`predict.rs:570-738`）

`SharedFactorStats` の同じラベル・同じ集計 FactorStat を読み `HorseExplanation` を返す純粋変換。

- 共有構造体を読むため「**根拠の数値 = score の入力**」が構造的に一致する（かつての手動同期欠陥を解消）
- `conditional_gate`（枠バイアス提示、#343）と `prev_run`（前走サマリ）は根拠固有のため本関数が自前で扱う。これらはスコアに投入しない（measure-first）
- `with_explanation=false` の通常 predict 経路では本関数を呼ばない（無駄な String 割当てを避けるため）

## #409 以前との違い（解消済み欠陥）

旧実装では `build_factors`（`predict.rs:437` 付近）と `build_explanation`（`predict.rs:528` 付近）が同一馬に対し同じラベル・同じ stat 行へ `stat_to_triple_opt` を **10 スロット独立に二重評価**していた。`build_explanation` の doc コメントが「factor を増やす際は両方を更新」「recency 有効化時に数値がズレる（既知の乖離点）」と手動同期の欠陥を自認していた。#409 の `SharedFactorStats` 導入でこの二重実装を解消した。

## 制約・注意事項

- `conditional_gate_stats` は `with_explanation=true` のときのみ取得する（確率のみの経路では DB クエリを発行しない）
- 相性 factor（jockey_venue / jockey_distance / jockey_horse_combo / horse_venue）は **本番 weight = 0** で挙動不変（#350 measure-first）。`SharedFactorStats` には値が入り、根拠提示には使われる
- `SharedFactorStats` のフィールドは `pub(crate)` 構造体内の非 pub フィールドのため、`predict.rs` の crate 外からは直接アクセスできない
