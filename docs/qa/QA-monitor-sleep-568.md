# QA — 監視プロセスのスリープ耐性（#568）

一次資料: [docs/original-docs/568-monitor-sleep-gap.md](../original-docs/568-monitor-sleep-gap.md)

## Q1: 監視が止まったのは「タイマーが飛んだ」のか「ループが終わった」のか

- 観測/根拠: プロセスは翌朝まで生存（issue 本文）。`run_monitor_loop`（`src/interface/monitor-loop/src/driver.rs`）は
  スイープ後 `tokio::time::sleep(interval*60)` を **1 回だけ** 呼んで次巡へ進む構造だった。
  終了ログ（`── 監視終了: …`）は一切出ていない。
- 回答: **確定。ループは終わっていない。長い単発 sleep の中で止まっていた**。
  終了メッセージが無いこと＝ break していないことなので、「終了判定が誤って終わった」線は消える。
  ホストが Standby / DarkWake を繰り返す間、この単発タイマーが満了せず次スイープに到達しなかった。
- 反映先: docs/knowledge/monitor-loop-sleep-resilience.md

## Q2: 復帰しても再開しないのはなぜか。復帰を検知する仕組みはあったか

- 観測/根拠: `driver.rs` に wall-clock（`Local::now()`）と待機の対応を取る箇所は無く、
  経過時間を測るコードも無かった。スイープが途切れたことをログに出す経路も無い。
- 回答: **確定。復帰検知の仕組みが存在しなかった**。単発 sleep は「残りいくら待つか」しか持たず、
  現実の経過時間と突き合わせない。したがって (a) 復帰しても即座に catch-up せず、
  (b) 途切れた事実がログにも残らない（＝沈黙）。
  沈黙は運用上「妙味が無かった」と誤読されるため、**警告を出すこと自体が要件**。
- 反映先: docs/knowledge/monitor-loop-sleep-resilience.md / ADR 0072

## Q3: 「最終レース発走から 14 時間経っても自動終了しない」の原因は何か

- 観測/根拠: `monitor_loop::classify(now: NaiveTime, post_time: Option<NaiveTime>, …)` は
  **時刻だけ**を比較し、日付を持たない。`driver.rs` も `let now = Local::now().time();`。
  `should_continue` は `Due | NotYet` が 1 つでもあれば true を返す。
- 回答: **確定。日付跨ぎで発走状態が巻き戻る**。翌日 08:55 の `now` は前日の全 post_time より前なので、
  昨日のレースが再び「発走前（Due/NotYet）」と判定され、`should_continue` が永久に true を返す。
  これは Q1 のタイマー停止とは**独立した第 2 の欠陥**で、仮にタイマーが正常でも終了しない。
- 反映先: docs/knowledge/monitor-loop-sleep-resilience.md / ADR 0072

## Q4: `caffeinate -i` を監視プロセスが自前で持てば 8/1 の事象は防げたか

- 観測/根拠: `pmset -g log` に `2026-08-01 13:23:32 Entering Sleep state due to 'Clamshell Sleep'`。
  以降は Standby からの 7 秒 DarkWake の反復。`scripts/predict-check/keep_awake.sh` の既存コメントにも
  「caffeinate はアイドルスリープのみ抑止。クラムシェル/`pmset` スケジュールスリープは止められない」とある。
- 回答: **確定。防げない**。8/1 は蓋を閉じたことによる clamshell sleep が起点で、`caffeinate -i` の
  守備範囲外。したがって **caffeinate は本件の根治策ではなく、アイドルスリープ分の露出を減らす
  best-effort に留まる**。根治は「寝ても壊れないループ」（Q1/Q2/Q3 の修正）側にある。
  それでも自前で持つ価値はある: 既存の keep-awake（#264）は launchd ジョブで開催日朝の
  `install.sh` が要り、install 忘れで無防備になる。監視プロセス自身の生存期間に紐付ければ運用手順に依存しない。
- 反映先: docs/knowledge/monitor-loop-sleep-resilience.md / ADR 0072 / deployments/launchd/README.md

## Q5: 修正が実機のスリープを跨げることをどう証明したか

- 観測/根拠: [一次資料 §3](../original-docs/568-monitor-sleep-gap.md) の 2026-08-04 実測ログ。
  本番と同じ `run_monitor_loop` を DB / netkeiba 非依存の fake Sweeper で 1 分間隔に回し、
  途中で `pmset sleepnow`。
- 回答: **確定**。15:49 就寝 → 16:20 復帰で
  途切れ警告が出て、**その直後にスイープ #3 が走った**（即時 catch-up）。
  以降は 1 分間隔へ復帰し正常終了。日付跨ぎ終了も実 DB の過去日指定で確認済み。
  なお `pmset sleepnow` は caffeinate を無視して寝るため、この検証は Q4 の「best-effort」とも整合する。
- 反映先: docs/knowledge/monitor-loop-sleep-resilience.md

## Q6: スイープ間隔を刻む幅（tick）はどう決めるか

- 観測/根拠: 最短スイープ間隔は predict-watch / odds-collect とも 1 分（CLI が 0 を弾く）。
  刻みを細かくすると復帰検知は速いが、寝ていない間の空回りが増える。
- 回答: **確定。30 秒**。最短間隔 1 分でも 2 ティックは刻める粒度で、既定間隔なら predict-watch
  5 分＝10 ティック / odds-collect 15 分＝30 ティック（いずれも無視できる負荷）。復帰検知の遅れは
  最大 30 秒で、発走直前 EV の判断粒度（分オーダー）に影響しない。
- 反映先: docs/knowledge/monitor-loop-sleep-resilience.md

## Q7: 日付跨ぎの終了判定はスイープの前と後、どちらに置くべきか

- 観測/根拠: セルフレビュー（PR #576）で 2 名の独立レビュアーが一致して指摘。odds-collect は
  `window()` が `None`（windowless）＝発走前なら常に `Due` なので、翌朝 08:55 に復帰した `now` では
  **前日の全レースが再び Due** になる。判定がスイープ後だと、終了する直前に全レースぶんの
  netkeiba 再スクレイプが走り `race_odds` / `race_odds_snapshots` に翌日タイムスタンプの行が積まれる。
  さらに `load_slots` の失敗経路は握って `continue` するため、末尾の判定を丸ごと迂回する。
- 回答: **確定。ループ先頭に置く**。当初は「1 スイープも走らずに終わる経路を作らない」意図でスイープ後に
  置いたが、(a) オッズ時系列を汚す、(b) DB 障害時に「終われない」経路が残る、の 2 点が上回る。
  過去日指定の誤用は起動時の `warn_if_not_today_jst` が別途警告するので、無言 no-op にはならない。
  `--once` は明示的な単発実行なので日付では止めない。
- 反映先: docs/knowledge/monitor-loop-sleep-resilience.md / ADR 0072

## Q8: 途切れはどの区間で測るべきか（待機区間 vs スイープ間隔）

- 観測/根拠: 当初実装は待機（sleep）区間の実経過だけを測っていた。predict-watch のスイープは
  `--scrape-delay`（既定 3000ms）× 対象レースぶん数分かかるため、**スイープ実行中にホストが寝ると**
  次の待機は想定どおりに見え、警告が出ないままになる。
- 回答: **確定。スイープ開始どうしの間隔で測る**。「止まったら必ず言う」を満たすには、監視が沈黙した
  区間全体を覆う必要がある。警告文言も実態に合わせて `⚠ 前回スイープから N 分空きました…` にした
  （N は前スイープ開始からの経過そのもの＝運用が知りたい沈黙の長さ）。
- 反映先: docs/knowledge/monitor-loop-sleep-resilience.md / ADR 0072
