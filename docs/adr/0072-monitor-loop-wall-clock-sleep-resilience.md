# 0072. 監視ループを wall-clock 基準にし、ホストのスリープに耐えさせる

## ステータス

承認済み（本 PR で実装）。対象 Issue: [#568](https://github.com/taito-station/paddock/issues/568)。

## コンテキスト

`paddock-predict-watch`（発走直前 EV 監視）と `paddock-odds-collect`（単複オッズ時系列収集）は、
[ADR 0060](0060-betting-axis-lock-preclose-topup.md) の「軸ロック＋ズレ増額」を実行するための
decision-support であり、**終日バックグラウンドで回り続けること**が前提の運用（keiba-start Step 5）。
両者はループ骨格 `monitor-loop`（#459 で共通化）を共有する。

2026-08-01、この前提が壊れた。監視は 14:32 のスイープを最後に沈黙し、
**14:50〜18:30 発走の約 12 レースが一度も評価されないまま**、翌朝までプロセスが生存し続けた
（生ログ: [docs/original-docs/568-monitor-sleep-gap.md](../original-docs/568-monitor-sleep-gap.md)）。

コードを読むと、独立した 2 つの欠陥があった。

1. **単発の長い sleep がホストのスリープを跨げない**。骨格は 1 スイープごとに
   `tokio::time::sleep(interval*60)` を 1 回呼ぶだけで、現実の経過時間（wall-clock）と突き合わせない。
   ホストが Standby / DarkWake を繰り返す間このタイマーが満了せず、次スイープに到達しなかった。
   復帰しても catch-up せず、**途切れた事実がログにも残らない**。
2. **終了判定が「時刻」しか見ていない**。発走状態判定 `classify(now: NaiveTime, post_time: NaiveTime, …)`
   は日付を持たない。日付を跨ぐと翌朝の `now` が全 post_time より前に戻り、昨日のレースが
   再び「発走前」と判定される。`should_continue` は永久に true を返し、**構造的に終われない**。

沈黙は運用上「妙味が無かった」と読まれる。**静かな失敗**であることが本件の最大の危険性で、
「途切れたら気づけること」も要件に含める。

なお macOS のスリープ抑止は既に `com.paddock.keep-awake`（#264・launchd ＋ `caffeinate -i`）が存在するが、
これは締切前 prefetch（#237）の launchd タイマーを回すためのジョブで、開催日の朝に `install.sh` を
叩く運用が要る。監視プロセスの生存期間とは無関係で、install 忘れがあれば監視は無防備になる。

## 決定

**監視ループの時間軸を単調タイマーから wall-clock へ移し、「寝ても壊れない」形にする。**
実装は共有骨格 `src/interface/monitor-loop/` に置き、predict-watch / odds-collect の双方を同時に直す。

1. **待機を wall-clock 期限に対する刻み待ちにする**。次スイープの期限（`Local::now() + interval`）を
   決め、30 秒刻みで現在時刻と期限を比べ直す。復帰時点で期限を過ぎていればループを抜ける
   ＝ **即座に次スイープが走る**（自動再開）。刻み幅 30 秒は最短スイープ間隔 1 分でも 2 ティックは
   刻める粒度（既定なら predict-watch 5 分＝10 ティック / odds-collect 15 分＝30 ティック）。
2. **途切れを検知して必ず警告する**。**スイープ開始どうしの実間隔**が
   `想定間隔 × 2 ＋ 前スイープの所要時間` を超えていたら `⚠ 前回スイープから N 分空きました…` を出す。
   待機区間ではなくスイープ間隔で測るのは、スイープ実行中に寝られたケースを取りこぼさないため。
   閾値に前スイープ所要を足すのは、スイープ自体が長い日（predict-watch は scrape_delay × 対象レース）に
   毎サイクル誤警告が出て警告が無視されるのを避けるため。
   **この警告は継続時だけでなく終了時にも通す**。日付を跨いで復帰したケースは 3 の終了経路に入るので、
   そこで黙ると「丸一日未監視だった日」が警告なしの正常終了に見える（＝潰したい静かな失敗そのもの）。
3. **wall-clock の日付で終了する**。`should_stop_by_date(対象日, JST の現在日)` を **ループ先頭**で評価し、
   対象日を過ぎたら時刻軸の判定に関わらず終了する。先頭に置くのは (a) 前日レースを再スクレイプして
   オッズ時系列を汚す 1 巡を作らないため、(b) `load_slots` が失敗し続ける経路（握って `continue`）でも
   必ず判定を通すため。現在日は**ホスト TZ ではなく JST** で取る（post_time が JST 起算。ホスト TZ だと
   JST より東のホストで開催途中に自己終了し、西のホストでは終了が最大 9 時間遅れる）。
   `--once` は「明示的に 1 スイープだけ回す」指定なので日付では止めないが、過去日なら警告する。
4. **監視プロセス自身がアイドルスリープを抑止する**。`run_monitor_loop` の冒頭で
   `caffeinate -i -w <自分の pid>` を spawn する（macOS のみ・`--once` では確保しない）。
   自プロセスを見張らせるので、監視がどう終わっても抑止が解放される＝解放忘れが構造上ない。
   既存の keep-awake エージェント（#264）は prefetch 用としてそのまま残す。

## 理由

- **wall-clock が唯一の正**。監視の目的は「発走時刻までに評価すること」であり、判断の基準は
  常に壁時計。単調タイマーはホストの電源状態という制御外の要因に左右されるので、
  時間軸を壁時計へ揃えるのが構造的な解になる。刻み待ちにすれば、OS がタイマーをどう扱ったかに
  関わらず「今が期限を過ぎているか」だけで正しく判断できる。
- **日付判定は必須で、代替が無い**。`classify` を日付付き（`NaiveDateTime`）に変えて根本から直す案も
  あるが、post_time は DB 上 `TEXT 'HH:MM'` で日付を持たず、当日運用では時刻比較で十分機能している
  （#459 で共通化した不変条件をそのまま活かせる）。**終了判定にだけ日付を足す**のが最小の変更で、
  既存の `classify` / `should_continue` のシグネチャと全テストを無傷に保てる。
- **caffeinate は根治策ではなく best-effort、と明示して採る**。8/1 の起点は 13:23 の
  `Clamshell Sleep`（蓋閉じ）で、`caffeinate -i` の守備範囲外＝**この修正があっても 8/1 は防げなかった**。
  根治は 1〜3（寝ても壊れないループ）側にあり、4 はアイドルスリープ分の露出を減らすだけ。
  それでも採るのは、既存 #264 の launchd 運用が「開催日朝の install」という人手の手順に依存しており、
  監視プロセスの生存期間に紐付ければその依存を外せるため。
- **骨格に入れることで両 app が同時に直る**。#459 で共通化した投資がここで効く。

## 却下した代替案

- **`classify` を日付付きに変える**。発走状態判定そのものを `NaiveDateTime` 化すれば日付跨ぎは
  原理的に起きないが、post_time が `TEXT 'HH:MM'` である以上どこかで「当日の日付」を合成する必要があり、
  責務が判定関数へ漏れる。既存の単体テスト群（windowed / windowless の境界）も総取り替えになる。
  終了判定にだけ日付を足せば同じ実害が消えるので、変更量に見合わない。
- **launchd の `StartCalendarInterval` で監視を定期起動する（常駐をやめる）**。スリープ中は launchd も
  発火せず（`deployments/launchd/README.md` の既知の限界）、同じ問題が残る。加えて毎回の起動コストと
  スイープ状態の分断が増える。
- **`pmset` の wake スケジュールを持たせる**。sudo が要り、ホストの電源設定を書き換える副作用が
  ユーザの他の用途に及ぶ。監視のために OS 設定を触るのは影響範囲が広すぎる。
- **常時稼働ホスト（RasPi / VM）へ監視を移設する**。これが完全解だが構成変更が大きく、本 issue の
  スコープを超える。`deployments/launchd/README.md` に既存の課題として残っており、本決定はそれと排他ではない
  （移設しても wall-clock 基準の耐性は無駄にならない）。

## 影響

- **追加**: いずれも crate 内部（`pub(crate)` 以下）。`detect_sweep_gap`（途切れ判定・純関数）/
  `should_stop_by_date`（日付終了判定・純関数）/ `capped_wait`・`minutes_or_max`（chrono の
  範囲外 panic 回避）/ `jst_date`（JST 基準の現在日）/ `keep_awake` モジュール（macOS の caffeinate 確保）。
  `driver` 内に `sleep_until_with_tick` / `next_deadline` / `wait_until_next_sweep` / `sweep_gap_notice`。
  predict-watch / odds-collect が使う公開 API は従来どおり `Sweeper` と `run_monitor_loop` だけ。
- **不変**: `classify` / `should_continue` / `count_started_before_post` / `Sweeper` トレイトの
  シグネチャは変えない。predict-watch / odds-collect 側のコード変更は無い（骨格の差し替えのみで効く）。
- **新しい終了経路**: 対象日を過ぎると
  `── {監視|収集}終了: 対象日（YYYY-MM-DD）を過ぎました。…` を出して終了する。
  過去日を `--date` に渡すと（`--once` 以外は）**1 スイープも走らずに終了する**（従来は無限ループしていた）。
  誤用そのものは起動時の `warn_if_not_today_jst` が伝える。
- **トレードオフ**: 時間軸を wall-clock に移したことで、NTP のステップ補正や手動の時刻/TZ 変更に
  感度を持つ（前進すれば偽の途切れ警告、後退すれば待機延長）。単調タイマーにはこの弱点が無かったが、
  ホストのスリープで監視が丸ごと死ぬ方が実害が大きいので受け入れる。
- **副作用**: 非 `--once` の起動時に `caffeinate` 子プロセスが 1 つ増える（macOS のみ・`/usr/bin` 固定・
  `env_clear` 済み）。監視の終了で自動的に消える。非 macOS / `caffeinate` 不在では no-op。
  外部から kill された場合はスイープごとの `try_wait` で検出して 1 度だけ警告する
  （抑止が静かに失われないようにする）。
- **検証**: 実機の `pmset sleepnow` を跨ぐ手動テスト
  `cargo test -p monitor-loop -- --ignored --nocapture crosses_real_host_sleep` を同梱した
  （DB / netkeiba 非依存。CI では `--ignored` でスキップされるがコンパイルは通るので腐らない）。
