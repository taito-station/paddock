# 568 — 監視プロセスがホストのスリープで止まった生ログ（一次資料）

`paddock-predict-watch` / `paddock-odds-collect` が macOS のスリープを跨げず、復帰後も再開しないまま
残り全レースが未監視になった件（[#568](https://github.com/taito-station/paddock/issues/568)）の生素材。
**書き換えない**（蒸留は [docs/knowledge/monitor-loop-sleep-resilience.md](../knowledge/monitor-loop-sleep-resilience.md) 側）。

## 1. 2026-08-01 の事象（発生時の観測）

- `predict-watch --date 2026-08-01` を 12:06 頃に nohup 起動
- 最終スイープは **14:32**（3 レース処理・正常終了して次サイクル待ちに入った）
- 以降、**14:50〜18:30 発走の約 12 レースが一度も評価されていない**
  （札幌 10-12R / 中京 8-12R / 新潟 8-12R）
- 翌 08:55 時点でプロセスは生存したまま。最終レース発走から 14 時間以上経過しても
  「全レース発走で自動終了」が働かず、終了ログも出ていない

## 2. `pmset -g log` 抜粋（2026-08-01）

```
2026-08-01 11:26:53 +0900 Wake      Wake from Standby [CDNVA] : due to EC.LidOpen/UserActivity Assertion Using BATT (Charge:58%) 6999 secs
2026-08-01 13:23:32 +0900 Sleep     Entering Sleep state due to 'Clamshell Sleep':TCPKeepAlive=active Using AC (Charge:83%) 6 secs
2026-08-01 13:23:38 +0900 DarkWake  DarkWake from Deep Idle [CDNP] : due to EC.ARPT/Maintenance Using AC (Charge:83%) 45 secs
2026-08-01 13:24:23 +0900 Sleep     Entering Sleep state due to 'Maintenance Sleep':TCPKeepAlive=active Using AC (Charge:83%) 1801 secs
2026-08-01 14:38:22 +0900 Sleep     Entering Sleep state due to 'Maintenance Sleep':TCPKeepAlive=active Using Batt (Charge:100%) 2757 secs
2026-08-01 15:24:19 +0900 DarkWake  DarkWake from Deep Idle [CDN] : due to EC.RTC/Maintenance Using BATT (Charge:100%) 7 secs
2026-08-01 15:24:26 +0900 Sleep     Entering Sleep state due to 'Maintenance Sleep':TCPKeepAlive=active Using Batt (Charge:100%) 3653 secs
2026-08-01 16:25:19 +0900 DarkWake  DarkWake from Deep Idle [CDN] : due to EC.RTC/Maintenance Using BATT (Charge:100%) 7 secs
2026-08-01 16:25:26 +0900 Sleep     Entering Sleep state due to 'Maintenance Sleep':TCPKeepAlive=active Using Batt (Charge:100%) 3654 secs
2026-08-01 17:26:20 +0900 DarkWake  DarkWake from Deep Idle [CDN] : due to EC.RTC/Maintenance Using BATT (Charge:100%) 7 secs
2026-08-01 17:26:27 +0900 Sleep     Entering Sleep state due to 'Maintenance Sleep':TCPKeepAlive=active Using Batt (Charge:100%) 3618 secs
2026-08-01 18:26:45 +0900 DarkWake  DarkWake from Deep Idle [CDN] : due to EC.RTC/Maintenance Using BATT (Charge:100%) 7 secs
2026-08-01 18:26:52 +0900 Sleep     Entering Sleep state due to 'Maintenance Sleep':TCPKeepAlive=active Using Batt (Charge:100%) 3653 secs
```

読み取れる生事実:

- **13:23 に `Clamshell Sleep`**（蓋を閉じた）。以降このホストは Standby / Deep Idle 系に入っている。
- 14:38 以降の復帰は**すべて `DarkWake` の 7 秒**（`EC.RTC/Maintenance`）で、直後に再び Sleep。
  ユーザ操作による本格復帰（`Wake from ... UserActivity`）は翌朝まで一度も無い。
- 発走時間帯（14:50〜18:30）はまるごとこのパターンの中にある。

## 3. 2026-08-04 の再現・検証ログ（修正後）

修正後のループ骨格（`monitor-loop`）を DB / netkeiba 非依存の手動テスト
（`cargo test -p monitor-loop -- --ignored --nocapture crosses_real_host_sleep`・1 分間隔）で回し、
途中で `pmset sleepnow` を実行して実スリープを跨がせた生ログ:

```
── アイドルスリープ抑止を確保しました（caffeinate -i -w 22817）。監視の終了で自動解放されます。蓋閉じ / pmset スケジュールスリープは抑止できません。
[15:48:50] スイープ #1
--- pmset sleepnow at 15:49:14 ---
Sleeping now...
[15:49:50] スイープ #2
⚠ スイープが 30 分途切れました（想定 1 分間隔・ホストのスリープ/停止の可能性）。直ちに再スイープします。途切れていた間に発走したレースは評価されていません。
[16:20:48] スイープ #3
[16:21:48] スイープ #4
[16:22:48] スイープ #5
── 監視終了: 本日（2026-08-04）は対象開催がありません。
```

同日、実 DB に対する過去日指定の生ログ（`--window 1` で対象 0 件＝スクレイプなし）:

```
⚠ --date 2026-08-01 は本日（2026-08-04）と異なります。…
── アイドルスリープ抑止を確保しました（caffeinate -i -w 10419）。…
── 15:36 スイープ: 対象 0 レース（窓 1分 / 🔶買い妙味≥100% ・ 🔍検証候補≥70%・判定は手動精査）
── 監視終了: 対象日（2026-08-01）を過ぎました。発走状態は時刻のみで判定するため、日付を跨いだら終了します。
```

`caffeinate` は監視プロセスの起動前と終了後のいずれも `pgrep -f "caffeinate -i -w"` が空で、
稼働中だけ存在し、監視プロセスの終了と同時に消えている。

> 注: 上の 2 ブロックは取得時点（セルフレビュー前のビルド）の観測。PR #576 のレビューを受けて
> 2 点変わったが、一次資料は観測時のまま残す（RO）:
> (1) 計測区間を「待機」から「スイープ開始どうし」へ変えたため、警告文言は
> `⚠ 前回スイープから N 分空きました…` になった。
> (2) 日付跨ぎの終了判定をループ先頭へ移したため、過去日指定では**スイープが 1 回も走らず**に
> 終了する（上の 2 ブロック目にある `── 15:36 スイープ: 対象 0 レース` の行は出なくなる）。
