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

## Q9: 日付跨ぎで終了するとき、途切れ警告は出すべきか

- 観測/根拠: セルフレビュー 2 巡目（PR #576）で 2 名の独立レビュアーが一致して [Must-fix]。
  Q7 で日付判定をループ先頭へ移した結果、**#568 の実事象そのもの（14:38 就寝 → 翌朝復帰）では
  復帰後に日付判定で即 break し、途切れ警告のブロックに一度も到達しない**。出力は
  `── 監視終了: 対象日（…）を過ぎました` だけで、`.claude/skills/keiba-start/SKILL.md` は
  この終了理由を「前日から回しっぱなしなら正常な後始末」と説明している。つまり
  **丸一日未監視だった日が「正常終了・警告なし」に見える**。
- 回答: **確定。終了経路でも必ず出す**。判定位置の最適化（Q7）が、この PR が潰そうとしている
  静かな失敗を主要ケースで復活させていた。終了行の直前に空き時間と最終スイープ時刻を出す。
  教訓として、**「必ず警告する」は継続経路だけでなく全終了経路を通ることまで含めて設計する**。
- 反映先: docs/knowledge/monitor-loop-sleep-resilience.md / ADR 0072 / keiba-start SKILL.md

## Q10: 途切れ判定の閾値は「想定間隔 × 2」のままでよいか

- 観測/根拠: 計測区間をスイープ開始どうしに変えた（Q8）ため、実間隔は
  `待機 ＋ スイープ所要` になる。predict-watch のスイープは `--scrape-delay`（既定 3000ms）×
  対象レース × 券種ぶんかかるので、**スイープ所要が interval を超える日は毎サイクル誤警告**になる。
- 回答: **確定。閾値に前スイープの所要時間を足す**（`想定間隔 × 2 ＋ 前スイープ所要`）。
  CLAUDE.md がこの警告行を「その日の判断材料が欠けている根拠」に格上げした以上、誤検知が常態化すると
  シグナルの価値が失われる。
- 反映先: docs/knowledge/monitor-loop-sleep-resilience.md / ADR 0072

## Q11: 日付判定の「現在日」はホストのタイムゾーンでよいか

- 観測/根拠: `post_time` は JST 起算だが、判定は `Local::now().date_naive()`（ホスト TZ）だった。
- 回答: **確定。JST で取る**（`lib.rs` の `JST_OFFSET_SECS` を使う `jst_date`）。ホスト TZ のままだと
  JST より東のホスト（UTC+10 以降）で**開催日の途中に日付が変わって自己終了**し、西のホスト（UTC 等）
  では終了が最大 9 時間遅れる。`warn_if_not_today_jst` は起動時に警告するだけで挙動は正さないので、
  判定側で吸収する。
- 反映先: docs/knowledge/monitor-loop-sleep-resilience.md / ADR 0072

## Q12: 閾値に「前サイクル所要」を足すと、測る区間を広げた意味は残るか

- 観測/根拠: セルフレビュー 3 巡目（PR #576）の代数的指摘。Q8 で計測を「スイープ開始どうし」に広げ、
  Q10 で閾値に前スイープ所要を足した結果、
  `実間隔 = 所要 + 待機` / `閾値 = 2×interval + 所要` となり、発火条件が **`待機 > 2×interval` と
  厳密に等価**に退化していた。例: interval 5 分、14:00 開始のスイープ中に 4 時間寝て 18:00 に完走
  → 実間隔 245 分 / 閾値 250 分 → **4 時間の沈黙が無警告**。Q8 が潰したはずのケースが復活していた。
- 回答: **確定。所要は単調時計（`std::time::Instant`）で測る**。単調時計はホストのスリープ中に
  進まないので、スリープぶんは壁時計の実間隔にだけ現れ、閾値には吸われない。これで
  「長いサイクルの誤警告を防ぐ（Q10）」と「スイープ中のスリープを検知する（Q8）」が両立する。
- 反映先: docs/knowledge/monitor-loop-sleep-resilience.md / ADR 0072

## Q13: JST 基準の判定を入れたら、テストは何に気をつけるか

- 観測/根拠: Q11 で終了判定を `jst_date` に変えたが、テスト側は `Local::now().date_naive()`
  （ホスト TZ）で対象日を作ったままだった。CI は `ubuntu-latest`・TZ 未設定＝ UTC なので、
  **JST 00:00〜09:00（UTC 15:00〜24:00）の 9 時間はホスト日 < JST 日となりテストが確実に落ちる**。
  実際に run `31132072607` の `ci` job が `当日なのに監視が終了した` で failure（27 passed / 1 failed）。
  ローカル（JST）では窓外で通っていたため気づけなかった。
- 回答: **確定。時刻に依存するテストは (a) 対象日を `jst_date` で作る、(b) 起点を JST の絶対時刻で
  アンカーする、(c) `TZ=` を変えて複数タイムゾーンで流す**。本 PR では
  `Asia/Tokyo` / `UTC` / `America/New_York` / `Pacific/Auckland` / `Europe/London` の 5 つで green を確認した
  （この確認で、3 巡目に追加した回帰テスト自身が同じ罠を踏んでいたことも判明した）。
- 反映先: docs/knowledge/monitor-loop-sleep-resilience.md

## Q14: 一次資料に「その後の実装変更」の注記を書いてよいか

- 観測/根拠: 3 巡目レビューで 2 名が指摘。`docs/original-docs/568-monitor-sleep-gap.md` は冒頭で
  「**書き換えない**」と宣言する RO 一次資料なのに、末尾にレビュー後の実装差分を説明する注記を足していた。
- 回答: **確定。書かない**。一次資料は観測ログのみに保ち、解釈・訂正・その後の変更は qa / knowledge
  の責務（CLAUDE.md の 3 層規約）。注記は本ファイル（Q8 / Q9）へ移した。一次資料に残る
  `⚠ スイープが 30 分途切れました` / `── 15:36 スイープ: 対象 0 レース` は**観測時点の出力**であり、
  現行実装では前者が `⚠ 前回スイープから N 分空きました`、後者は出力されない（判定がループ先頭のため）。
  同じログの `── アイドルスリープ抑止を確保しました（caffeinate -i -w 10419）` も出力されない
  （抑止の確保を日付判定の**後**へ移したため、過去日指定では確保に到達しない）。
- 反映先: docs/original-docs/568-monitor-sleep-gap.md（注記を除去）
