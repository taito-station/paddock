# QA — predict-watch のゲート通過通知（#584）

一次資料: [docs/original-docs/584-predict-watch-notification.md](../original-docs/584-predict-watch-notification.md)

## Q1: macOS 通知を鳴らす閾値は何にするか。既存の `--notify-gate` を流用できるか

- 観測/根拠: `--notify-gate`（`src/apps/predict-watch/src/cli.rs:33-39`）は **🔍 の表示閾値**で、
  「notify_gate 以上・roi_gate 未満を検証候補として表示に残す」（#345）ためのもの。名前は通知だが
  ベルとは無関係。一方 issue #584 の要件は「ゲート（既定 ROI ≥ 100%）を通過したレース」の通知で、
  ADR 0076 の実測ではその閾値の通過は 182R / 839 スイープで 0 件（本一次資料でも 246 スイープ 0 件）。
- 回答: **確定。専用フラグ `--notify-roi` を新設し、既定は `--roi-gate` と同値**にする。
  `--notify-gate` は流用しない——流用すると #345 の 🔍 帯（表示）と発火の意味が潰れ、
  「表示を増やしたつもりが鳴り出す」「鳴らすつもりで表示だけ増える」の両方向の誤用を生む。
  専用フラグにすることで、#571 のゲート較正が済むまでは手で下げて実地検証できる。
  `--roi-gate` との大小は制約しない（下げるのが本来の用途で、`resolve_notify_gate` が
  `> roi_gate` を弾く理由＝「🔍 帯が構造的に空になる」は発火閾値には存在しない）。
- 反映先: docs/knowledge/monitor-loop-sleep-resilience.md（規律 4・決定ログ #584）

## Q2: 既定閾値では鳴らないなら、この機能は何をしたことになるのか

- 観測/根拠: ADR 0076（182R / 839 スイープ）と本一次資料（4 開催日 246 スイープ）のいずれも
  🔶 の通過は 0 件。issue #584 自身も「本機能を入れても現状の閾値では発火しない可能性が高い」と認めている。
- 回答: **確定。経路と閾値は独立に解く**。#584 は経路（通ったら人に届く）、#571 は閾値（何を通すか）。
  経路が無いまま閾値だけ較正すると、較正した瞬間の判定がまたログに埋もれる。
  ただし「作ったのに鳴らない」を沈黙にしないため、**起動時に「既定では鳴らない」と毎回宣言する**
  注記を機能要件と同格で入れる。これが無いと「鳴らない＝妙味なし」と「そもそも既定では鳴らない」が
  区別できず、`monitor-loop-sleep-resilience.md` の前提が名指しする静かな失敗をそのまま作る。
- 反映先: docs/knowledge/monitor-loop-sleep-resilience.md / CLAUDE.md

## Q3: 通知本文の「レース名」は競走名まで引くか、既存ラベルで足りるか

- 観測/根拠: `race_label`（`watch.rs:365`）は `函館10R 15:35` 形式で、場・R・発走時刻を一意に特定できる。
  一方 `Slot` は競走名を持たない。日付一括で引く既存 API `Interactor::race_names_by_date`
  （`src/use-case/src/interactor/race/races_by_date.rs:26`）があり、post_time / race_class と同型で
  N+1 にならない（REST の `handler/race.rs:111` が先行利用）。
- 回答: **確定（ユーザー判断）。競走名も引く**。`load_slots` に 3 本目の日付一括クエリを足し、
  `函館10R 15:35 巴賞 ・ …` の形にする。未保存（平場）は None でラベルのみに縮退する。
  use-case / rdb-gateway の変更はゼロ。
- 反映先: src/apps/predict-watch/src/watch.rs（`Slot` / `load_slots`）

## Q4: 同一レースの連投をどう抑制するか

- 観測/根拠: 2026-08-09 の新潟10R 17:20 は 7 スイープ連続で通過帯に入り、参考ROI は
  73.9 → 80.3 → 80.2 → 79.9 → 79.1 → 79.1 → 76.1 と推移した（振れ幅 6.4pt）。
  抑制が無ければこの 1 レースだけで 7 連投。
- 回答: **確定（ユーザー判断）。レースごと初回は必ず通知し、以降は前回通知時の参考ROI から
  +10pt 上振れしたときだけ再通知する**。上の実測列に当てると 7 通知が 1 通知に落ちる
  （初回 73.9 の次に鳴るには 83.9 以上が要る）。記録するのは「**通知した**ときの ROI」で、
  閾値未満で見送った pass は状態を汚さない——汚すと次に本当に通過したとき鳴らなくなる。
- 反映先: src/apps/predict-watch/src/notify.rs（`select_notifications` / `should_notify`）

## Q5: 抑制状態を永続化するか

- 観測/根拠: 既存 shell の連投防止は marker ファイル（`~/Library/Logs/.snapshot-coverage-done.<DATE>`）で
  日単位の冪等化。ただしこれは「1 日 1 回でよい報告」であって、ROI のような連続値の抑制ではない。
- 回答: **確定。永続化しない（プロセス内メモリのみ）**。抑制は「1 プロセスの連投抑止」であって
  永続契約ではなく、永続化すると監視の再起動後に**本当に必要な初回通知を落とす**（＝届かない）側に倒れる。
  #584 の障害は「届かない」ことなので、失敗するなら鳴りすぎる側へ倒す。
  再起動で抑制がリセットされる点は keiba-start の「当日分の監視は落とさない」運用が実質の緩和策。
- 反映先: docs/knowledge/monitor-loop-sleep-resilience.md（決定ログ #584 の影響）

## Q6: 配送手段は何を使うか。Rust から osascript を叩くのは second source ではないか

- 観測/根拠: Rust ワークスペースに通知実装は無く（`Command::new("osascript")` も通知クレートも 0 件）、
  shell 4 本が同一形の `notify()` を持つ。`deployments/launchd/README.md:29` は
  「osascript 通知は表示セッション依存でベストエフォート。ログのマーカーが一次情報」と定めている。
- 回答: **確定。osascript（`display notification`）を Rust から同じ呼び出し形で叩く**。
  ADR 0064 が戒める second source は「同じ判断を二重実装すること」であって、同じ配送機構を
  別言語から使うことではない。実利は権限にある——osascript への通知許可は #493 で既に付与済みで、
  `notify-rust` 等を入れると別 bundle の許可が要り、**未許可のまま無言で鳴らない**＝#584 が
  問題にした「届かない」を新しい形で再生産する。呼び出し形（argv 渡し・失敗の握り潰し・
  `paddock <機能>` の title 命名・ログ側マーカーの一次情報化）も既存 4 本に揃える。
- 反映先: docs/knowledge/monitor-loop-sleep-resilience.md（規律 4）/ src/apps/predict-watch/src/notify.rs

## Q7: 通知の有効/無効の既定はどちらか

- 観測/根拠: issue の要件は「通知の有効/無効はフラグで切れること（cron / `--once` 検証時に鳴らさない）」。
- 回答: **確定。既定は有効で `--no-notify` で切る**。#584 の障害は「届かない」ことなので、
  既定を無効にすると同じ穴が残る。`--once` / cron でも既定は有効のまま——ここだけ自動で
  黙らせると「cron だけ沈黙する」という新しい静かな失敗を作る。鳴らしたくない検証では
  明示的に `--no-notify` を渡す。
- 反映先: src/apps/predict-watch/src/cli.rs
