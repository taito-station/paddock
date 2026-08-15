# QA — `--overview` の発走時刻・発走済み表示（#587）

一次資料: [#587](https://github.com/taito-station/paddock/issues/587)（転記しない・ADR 0074）。
本文は `gh issue view 587` で取得する。

## Q1: 発走時刻・発走済み表示を出す範囲は `--overview` だけか

- 観測/根拠: 見出しのフォーマット文字列は `src/apps/predict/src/session.rs` の `run_race`
  （対話 / `--skip-all`）と `run_overview` に**同一のものが 2 か所重複**していた。
  `--skip-all`（#479）も「朝に EV 一覧を俯瞰する」用途で常用しており、#587 が報告した
  「上位に並んだレースが既に終わっているか読めない」誤読は同じ形で起こる。
- 回答: **確定。全経路に出す**（ユーザー確認済み）。見出しを純関数 `race_heading` に抽出して
  両経路で共有し、重複による drift も同時に潰す。
- 反映先: ADR 0085 / docs/specifications/predict-session.md

## Q2: 発走済みをどう表示するか（未発走側にもマークを付けるか）

- 観測/根拠: 候補は (a) 発走済のみ `[発走済]`、(b) `[発走済]`/`[未発走]` の両側マーク、
  (c) 時刻のみでマーク無し。#587 の要件は「発走済みを明示」。
- 回答: **確定。(a) 発走済のみマーク**（ユーザー確認済み）。発走時刻は常に出し、`post_time` 不明は
  `（発走 --:--）` としてマークを付けない。(b) は全行が常時賑やかになる割に情報が増えない、
  (c) は要件を満たさない。
- 反映先: ADR 0085 / docs/specifications/predict-session.md

## Q3: 「発走済み」の判定基準は SPA の ⚫終（`result_confirmed`）と揃えるか

- 観測/根拠: SPA は「終了」を**結果確定**で判定し、`post_time` 経過だが未確定のレースは未発走側に
  残す（#381・`docs/specifications/web-spa.md`）。一方 #587 の要件は「実行時刻を基準に発走済みを明示。
  判定は `race_cards` の `post_time` を一次ソースにする（#391 と揃える）」。
  用途も違う——SPA は結果の反映を待つ画面、CLI は「今から張れるか」を見る出力。
- 回答: **確定。揃えない。CLI は `post_time` 経過で「発走済」とする**。結果が未確定でも
  張れないことに変わりはないため、待つ意味がない。同じ語（終了 / 発走済）で別基準になるので、
  出力に「[発走済] は実行時刻に発走済み（結果確定の有無とは別）」の注記を 1 行添える。
- 反映先: ADR 0085 / docs/specifications/predict-session.md

## Q4: 発走状態の判定ロジックを新規に書くか（second source を作らないか）

- 観測/根拠: `monitor_loop::classify(now, post_time, has_result, window)`
  （`src/interface/monitor-loop/src/lib.rs`）が純関数として既にあり、`window: None` で
  「発走済みか否か」に落ちる。ただし **classify は `NaiveTime` のみで日付を持たない**
  （日付跨ぎは監視側の `should_stop_by_date` が別担当・#568/QA-monitor-sleep-568 の Q3）。
  `--overview` は過去日も指定できるため、classify だけでは過去日のレースが「未発走」に見える。
- 回答: **確定。時刻軸は classify に委譲し、predict 側には日付軸だけを畳む薄いラッパ
  `is_started_at` を置く**（過去日→発走済 / 未来日→未発走 / 当日→classify）。判定の正本は
  classify のままなので second source は増えない。`predict` から `monitor-loop` への依存追加は
  apps → interface で規約上の正方向。
- 反映先: ADR 0085

## Q5: 発走済みレースを一覧から除外しないか

- 観測/根拠: `--overview` は「完了済みセッションでも当日オッズで EV 一覧を再計算して表示する」
  （#551）＝**事後の見返しを含む**のが仕様。除外すると過去日の overview が空になる。
- 回答: **確定。除外せず区別する**（#587 の要件どおり）。
- 反映先: ADR 0085

## Q6: ドキュメントはどこまで回すか

- 観測/根拠: 表示の追加に見えるが、Q3（SPA と基準を意図的に分ける）と Q5（除外せず区別）は
  決定を伴う。CLAUDE.md は「決定を伴う変更は ADR を起票する」。
- 回答: **確定。ADR 起票 + `docs/specifications/predict-session.md` への写し**（ユーザー確認済み）。
  knowledge / specifications の増減は無いので `doc-classes.md` は触らない。
- 反映先: ADR 0085 / docs/specifications/predict-session.md
