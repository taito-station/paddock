# 774 prefetch のレース選択失敗の実測（2026-10-12）

`scripts/predict-check/prefetch_odds.sh` が「レース選択コマンドに失敗」で中断していた件の調査ログ。
本体ログ `~/Library/Logs/paddock-prefetch.log`（2026-08-08 から記録・#493）と、Claude Code のセッション記録、
調査時点のホストの状態から分かったことを残す。

## 1. 失敗の分布（本体ログの集計）

- 「レース選択コマンドに失敗」は計 6,063 行。最初は 2026-08-08T10:06:32、最後は 2026-10-03T11:33:53。
- 2026-08-10〜08-14 のような平日（開催の無い日）にも、1 日 140〜230 回出ている。対象の有無と関係なく、選択の python が毎回
  非 0 で終わっていた。1 日 288 回（5 分おき 24 時間）に届かない日は、ホストがスリープしていた時間の分だけ少ない。
- 失敗から成功に変わったのは 2026-09-27 の 1 回だけ。16:25:38 の実行までは失敗で、16:30:56 の実行で `prefetch 開始` に進み、
  以後は成功した。
- 2026-10-03 は成功 58 回・失敗 1 回。失敗の 1 回の直前には `FATAL: sorry, too many clients already` が出ている
  （共有 DB の接続上限。#717 の系統）。

## 2. 原因の手がかり

- 同じ本体ログに、snapshot-coverage（`snapshot_coverage_check.sh` → `snapshot_coverage.py`）が
  `ModuleNotFoundError: No module named 'zoneinfo'` を 449 回残している。期間は 2026-08-08T18:42〜08-30T23:59。
  snapshot-coverage は prefetch と同じ launchd の PATH（`/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin`）で
  `python3` を呼ぶ。
- `upcoming_races_db.py` も先頭で `from zoneinfo import ZoneInfo` を import する。同じ python3 なら、選択も import の段階で
  毎回落ちる。
- prefetch の選択の stderr は、`prefetch_odds.sh` が本体ログに写さず launchd の `StandardErrorPath`（`/tmp/paddock-prefetch.launchd.err.log`）
  に出ていた。このファイルは `/tmp` の掃除で消えていて、選択そのものの失敗理由は 1 行も残っていない。
- 2026-09 は snapshot-coverage が本体を一度も実行しておらず（全レース発走後の行が無い）、python3 の状態を示す行も無い。
  2026-09-27 16:25〜16:30 の切り替わりについて、セッション記録には launchd・brew・pyenv・ディスク掃除・DB の再起動の操作が無い。
  直前の 16:22 に外部ツール（Orca）が worktree を削除しているが、prefetch との関係は確かめられない。

## 3. launchd にロードされていた期間（セッション記録から）

- 2026-09-27 09:52 の `launchctl list` に `- 1 com.paddock.prefetch-odds`（ロード済み・最後の終了コード 1）がある。それより前の install の記録は無い。
- 2026-09-27 11:15 に primary の `install.sh` を実行し、16:38 に `uninstall.sh` で prefetch-odds・keep-awake・snapshot-coverage を外した。
- 2026-09-29・10-03 10:25 の `launchctl list` には常駐の 4 本（backup-db など）しか無い。
- 2026-10-03 10:46 に primary の `install.sh` を実行し（keiba-start の Step 1.6）、16:40 に `uninstall.sh` で外した。
  install の約 2 分前に `limactl start paddock` で DB の VM を起こしている。
- つまり 2026-08〜09-27 は入れっぱなしで、その後は開催日だけ入れる運用（設計どおり）になっている。10-03 の成功は
  「壊れていたものが直った」のではなく、入れ直して最初から成功したもの。

## 4. 調査時点（2026-10-12）のホスト

- `/usr/local/bin/python3` は Homebrew の python@3.14.8（2026-10-02 11:48 に入れ直し・`INSTALL_RECEIPT` の time も同時刻）を指す。
  それより前に python3 がどの版の実体を指していたかは、もう確かめられない。
- python@3.13（/usr/local/bin/python3.13）・python@3.14・/usr/bin/python3（Xcode の 3.9.6）はどれも `import zoneinfo` に成功する。
- site-packages に worktree を指す `.pth` や editable install は無い。
- launchd と同じ条件（`env -i` で HOME・PATH だけ与える）で
  `prefetch_odds.sh --date 2026-10-03 --at 12:00 --dry-run` を流すと、対象 2 レースを選んで exit 0。

## 5. 修正後の確認（2026-10-13）

- launchd と同じ条件で `PADDOCK_DB_URL` の port を 1 にして流すと、exit 1 になり、本体ログに次が残る。
  ```
  [2026-10-13T09:36:46+0900]   選択の stderr: psql: error: connection to server at "127.0.0.1", port 1 failed: Connection refused
  [2026-10-13T09:36:46+0900]   選択の stderr: 	Is the server running on that host and accepting TCP/IP connections?
  [2026-10-13T09:36:46+0900]   選択の stderr: DB から発走時刻を取得できません（詳細は直前の stderr）
  [2026-10-13T09:36:47+0900] レース選択コマンドに失敗（DB 接続不可・日付不正・クエリ失敗等）。中断する。（python3: /usr/local/bin/python3 → /usr/local/opt/python@3.14/bin/python3.14 3.14.8）
  ```
- 成功時は件数行（`# 2 races (of 19 carded with post_time)`）を stderr に出すだけで、本体ログには積まない。
- どちらの場合も、stderr を受けた一時ファイル（`/tmp/paddock-prefetch-select.*`）は残らない。
