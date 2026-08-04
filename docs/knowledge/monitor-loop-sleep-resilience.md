---
status: Confirmed
kind: knowledge
sources:
  - docs/adr/0072-monitor-loop-wall-clock-sleep-resilience.md
  - docs/adr/0060-betting-axis-lock-preclose-topup.md
  - docs/qa/QA-monitor-sleep-568.md
  - docs/original-docs/568-monitor-sleep-gap.md
distilled_from_sha: "18aed60"
updated: "2026-08-04"
---

# 監視ループのスリープ耐性（predict-watch / odds-collect）

終日バックグラウンドで回る 2 つの監視 app は、ループ骨格 `src/interface/monitor-loop/` を共有する。
ホスト（macOS ラップトップ）のスリープを跨ぐことが常態なので、**時間軸は wall-clock を正とする**。

## 前提: 何が壊れうるか

監視の失敗は**沈黙として現れる**。通知が来ないことは「妙味が無かった」と区別がつかないため、
止まっていたこと自体に気づけないのが最大のリスク（2026-08-01 に約 12 レースが未評価のまま流れた）。
したがって設計要件は「止まらないこと」だけでなく **「止まったら必ず言うこと」** を含む。

## 3 つの規律

### 1. 待機は wall-clock 期限への刻み待ちにする

次スイープの期限を `Local::now() + interval` で決め、**30 秒刻み**で現在時刻と期限を比べ直す
（`driver::sleep_until_with_tick`）。単発の長い `tokio::time::sleep` はホストのスリープ中に満了せず、
復帰後も「残り」を待ち続けて監視が無言で止まる。刻み待ちなら OS がタイマーをどう扱ったかに関わらず
「今が期限を過ぎているか」だけで判断でき、**復帰時点で期限を過ぎていれば即座に次スイープが走る**。

刻み幅 30 秒の根拠: 最短スイープ間隔は 1 分（両 app とも CLI が 0 を弾く）。通常運用の空回りは
1 スイープあたり高々 2 回で、復帰検知の遅れも最大 30 秒＝発走直前 EV の判断粒度（分オーダー）に響かない。

### 2. 途切れたら必ず警告する

待機の実経過を測り、想定間隔の **2 倍**を超えていたら `detect_sweep_gap` が検知して
`⚠ スイープが N 分途切れました…` を出す。ログにこの行があれば、その間に発走したレースは
評価されていない。**この警告が出ている日の「通知ゼロ」は妙味なしの根拠にならない**。

### 3. 終了は wall-clock の日付で決める

発走状態判定 `classify` は `NaiveTime` 同士の比較で**日付を持たない**。当日運用では十分だが、
日付を跨ぐと翌朝の `now` が全 post_time より前に戻り、昨日のレースが再び「発走前」に見える。
`should_continue` は永久に true を返し、**構造的に終われない**（実測: 最終レース発走から 14 時間経過後も生存）。

そこで継続判定に `should_stop_by_date(対象日, 現在日)` を併用し、対象日を過ぎたら終了する。
判定は**スイープ後**に置く（1 スイープも走らずに終わる経路を作らない）。

- 実務上の見え方: `--date` に過去日を渡すと **1 スイープだけ走って終了する**。
  `── {監視|収集}終了: 対象日（YYYY-MM-DD）を過ぎました。…` が出る。

## アイドルスリープ抑止は best-effort（根治策ではない）

非 `--once` 起動時、監視プロセスは自分で `caffeinate -i -w <自分の pid>` を確保する
（macOS のみ・`monitor_loop::keep_awake`）。自プロセスを見張らせるので、監視がどう終わっても
（正常終了・パニック・kill）抑止が解放される。

**限界を取り違えないこと**: `caffeinate -i` が止められるのはアイドルスリープだけ。

- **蓋を閉じれば寝る**（clamshell sleep）。2026-08-01 の事象はまさにこれが起点で、
  **この抑止があっても防げなかった**。
- `pmset` のスケジュールスリープ・強制スリープ（`pmset sleepnow`）も止められない。
- 既に寝ているホストを起こすこともできない。

つまり耐性の本体は上の規律 1〜3（寝ても壊れないループ）であり、caffeinate はアイドル分の露出を
減らすだけ。**外出中に監視を当てにするなら、蓋を閉じない**（または常時稼働ホストへ移設する。
[deployments/launchd/README.md](../../deployments/launchd/README.md) の既知課題）。

## 既存 keep-awake エージェント（#264）との棲み分け

| | 対象 | 生存期間 | 起動 |
|---|---|---|---|
| `com.paddock.keep-awake`（#264） | 締切前 prefetch（#237）の launchd タイマー | 当日の最終 post_time まで | 開催日朝に `deployments/launchd/install.sh` |
| `monitor_loop::keep_awake`（#568） | predict-watch / odds-collect 自身 | 監視プロセスの生存期間 | 監視の起動と同時（手順不要） |

前者は launchd ジョブなので **install を忘れると効かない**。後者は監視プロセスに紐付くため運用手順に
依存しない。両者は排他ではなく、開催日は両方効いていてよい（`keep_awake.sh` は lock + PID 生存で
多重起動を防ぐ）。

## 検証のしかた

実スリープを跨ぐ挙動は、DB / netkeiba 非依存の手動テストで確認できる:

```sh
cargo test -p monitor-loop -- --ignored --nocapture crosses_real_host_sleep
# 起動後に別端末で `pmset sleepnow`（caffeinate は強制スリープを止めないので寝る）。
# 復帰させて「⚠ スイープが N 分途切れました」＋直後のスイープが出れば OK。5 スイープで自動終了。
```

本番と同じ `run_monitor_loop` を 1 分間隔で回すので、骨格の変更はこのテストで実機検証できる。
自動テスト（`cargo test -p monitor-loop`）側には、期限超過時の即時復帰・刻み跨ぎの待機・
途切れ判定の境界・日付跨ぎ終了が入っている。
