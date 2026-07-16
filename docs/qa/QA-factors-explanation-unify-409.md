# QA: #409 build_factors / build_explanation の二重実装単一化

> 質問票+回答（[docs/qa/README.md](README.md)）。一次資料は issue #409（全体レビュー 2026-07-15 由来）。
> 関連確定知: [ADR 0014](../adr/0014-none-baseline-exclusion.md)（predict・backtest は同じ `build_factors`/`estimate_probabilities` を共有）、
> [ADR 0055](../adr/0055-ev-layer-separation-circular-break.md)（`predict_race_views` は factor 収集 1 回）。
> 回答確定後、純リファクタで新規決定を伴わなければ ADR 起票は不要。設計上の含意は knowledge へ蒸留する。

## Q1: 共有中間構造体に何を持たせ、recency の乖離点をどう扱うか

- 観測/根拠:
  - `build_factors`（predict.rs:437）と `build_explanation`（:528）が、同一馬に対し**同じラベル**（surf/dist/gate/venue/tc）で**同じ stat 行**へ `stat_to_triple_opt` を**10 スロット独立に二重評価**している（`collect_race_factors` の単一ループ内で連続呼び出し・predict.rs:240,258）。
  - build_explanation の doc コメントが「factor を増やす際は両方を更新」「recency 有効化時に数値がズレる（既知の乖離点）」と手動同期の欠陥を自認（predict.rs:520-526）。
  - build_factors のみ recency 有効時に horse 3 因子（surf/dist/tc）を時間減衰レートで上書き（:458-480）。**本番は `recency: None`**（config.rs production）なので現状は両者同値。
  - build_explanation のみ conditional_gate（枠バイアス）と prev_run を扱う（explanation 固有）。
- 回答: **確定（ユーザー決定 2026-07-16）。共有構造体 `SharedFactorStats` を新設**し、解決済みラベル（surf/dist/gate/venue/tc_opt）＋ 10 スロットの集計 `Option<FactorStat>` を保持。`collect_race_factors` のループで `resolve_shared_factors(...)` を**1 回だけ**評価し `&SharedFactorStats` を両関数へ渡す。
  - build_factors は共有 10 スロットを読み、**recency 有効時のみ horse 3 因子を上書き**（乖離点をこの 1 箇所に閉じ込め、doc の「両方更新」欠陥を解消）。
  - build_explanation は共有 10 スロット＋ラベルから `FactorExplanation` を組み、conditional_gate/prev_run は従来通り自前で扱う。
- 反映先: `src/use-case/src/interactor/race/predict.rs`、knowledge（scoring 経路の factor 収集構造）。

## Q2: too_many_arguments allow の対象範囲（issue のカウントと実態の乖離）

- 観測/根拠:
  - issue は allow 3 箇所（predict.rs:436 / :527 / `live/mod.rs:183`）を「同構造体で解消」と要求。
  - 実物確認: `src/use-case/src/interactor/live/mod.rs:183` の allow は **`#[cfg(test)] mod tests` 内のテスト補助 `snap`**（`LiveEvSnapshot` ビルダ・8 引数）で、**build_factors/build_explanation と無関係**。use-case 全体の allow は実際この 3 つのみ。
- 回答: **確定（ユーザー決定 2026-07-16）。predict.rs:436 / :527 の 2 箇所のみ本 PR で解消**（共有構造体の受け渡しで引数削減）。`live/mod.rs:183` は無関係なテスト補助のため**本 PR 対象外**（1 PR 1 トピック維持）。issue の 3 カウントは誤りとして PR で明記し、テスト補助の整理が要るなら別 issue。
- 反映先: issue #409 コメント / PR 本文で乖離を明記。

## Q3: 挙動不変の検証方法（unit test 代替か backtest 実行か）

- 観測/根拠:
  - 純リファクタ（計測順序ルール対象外）。同一入力・同一 `stat_to_triple_opt` を読む構造のため score 不変は構造的に自明。
  - 既存 unit test 15 本（build_explanation 中心・predict.rs:786-1361）＋全 cargo test が退行検知網。
  - backtest は実 DB（colima/Postgres）が必要だが**現在 colima 停止**。起動は Lima VM＋golden DB で重く、共有 DB 競合の懸念（memory: shared-db-contention）。
- 回答: **確定（ユーザー決定 2026-07-16）。既存 unit test ＋ 全 cargo test green を一次証跡**とし、構造的同値性で挙動不変を担保。backtest は挙動不変が自明なため今回は回さない（要望時に DB 起動して production フラグ `--shrinkage-m 10 --win-power 1.25 --place-show-power 2.0` で before/after の ROI 一致を確認できる）。
- 反映先: PR 本文の Test plan。
