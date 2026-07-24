# 締切前 live オッズ自動 prefetch（#237）＋ keep-awake（#264）＋ 日次 DB バックアップ（#265）＋ バックアップ鮮度監視（#490）＋ snapshot retention（#492）

6 つの launchd エージェントを `install.sh` でまとめて配置する:

- **prefetch（#237）**: 発走 N 分前のレースの最新オッズを定期取得し、`race_odds_snapshots`（#232）に
  締切前 live スナップショットを蓄積する。これが回ると #218（live オッズで α 再校正）や #248
  （年間 +EV 集計）の入力が貯まる。
  - 本体: [`scripts/predict-check/prefetch_odds.sh`](../../scripts/predict-check/prefetch_odds.sh)
  - レース選択: [`scripts/predict-check/upcoming_races_db.py`](../../scripts/predict-check/upcoming_races_db.py)
    （#235 の `race_cards.post_time` を DB 参照。netkeiba 都度スクレイプ無し）
- **keep-awake（#264）**: 開催日の発走ウィンドウ中、`caffeinate -i` で Mac のアイドルスリープを
  抑止し、上記 prefetch の 5 分タイマーが寝て止まる取りこぼしを防ぐ。
  - 本体: [`scripts/predict-check/keep_awake.sh`](../../scripts/predict-check/keep_awake.sh)
- **backup-db（#265）**: 毎日 23:30 に full DB dump を退避先へ退避＋世代管理する常駐エージェント。
  prefetch/keep-awake が開催日限定（朝 install・夜 uninstall）なのに対し**常駐**で、`uninstall.sh`
  では**外れない**（開催日夜に uninstall しても当夜のバックアップを守るため。詳細は下記と
  [`deployments/db/BACKUP.md`](../db/BACKUP.md)）。
  - 本体: [`scripts/backup-db.sh`](../../scripts/backup-db.sh)
- **backup-staleness（#490）**: バックアップの欠落日（「実行されなかった」日）を検知する鮮度監視。
  毎時 1 回（StartInterval=3600）＋ロード時（RunAtLoad=true）に発火し、最新 dump が
  36h を超えて古ければ osascript 通知とログ（STALE マーカー）で警告する。スリープ復帰時は
  StartInterval が coalesce して発火し catch-up 検知する（launchd はスリープ中の Mac を起こさないため
  検知は次に Mac が起きたとき）。backup-db の失敗通知（FAIL
  マーカー）は「実行されたが失敗した」場合にしか発報しないため、Mac スリープ/colima 停止による
  無言欠落は本 agent が補完する。backup-db と対になって常駐し、`uninstall.sh` では外れない。
  - 本体: [`scripts/backup-staleness-check.sh`](../../scripts/backup-staleness-check.sh)
  - ログ先: `~/Library/Logs/paddock-backup.log`（backup-db と同じファイルに集約）
  - 注意: osascript 通知は表示セッション依存でベストエフォート（launchd 配下では表示されないことがある）。
    ログの STALE/FAIL マーカーが一次情報。
- **verify-backup-restore（#474）**: 毎週日曜 04:00 に最新 dump を scratch DB へ復元し主要テーブルの
  行数を golden と突合する週次 restore 検証（「復元できない dump を守っていた」を検知）。backup-db と
  対になって**常駐**し、`uninstall.sh` では外れない。
  - 本体: [`scripts/verify-backup-restore.sh`](../../scripts/verify-backup-restore.sh)
  - ログ先: `~/Library/Logs/paddock-backup.log`（backup-db と同じファイルに集約）
- **purge-snapshots（#492）**: 毎日 04:30 に `race_odds_snapshots` の retention を適用する常駐
  エージェント。`race_odds_snapshots` は締切前 live オッズを 15 分毎に append する再取得不能資産だが
  ≈30MB/日・年 ≈11GB で単調増加するため、放置すると Colima VM ディスクと dump サイズ（backup 時間・
  off-machine 転送量に直結）が黙って肥大する。保持月数（既定 **6 ヶ月**）より古い snapshot を日次で
  削除して bounded に保つ。実行時刻 04:30 は backup-db（23:30）の**後**に置き、当夜の dump は purge 前の
  状態を退避してから翌朝に古い snapshot を削る順序にする。backup-db と対になって**常駐**し、
  `uninstall.sh` では外れない（retention は開催日に依らず日次で回す必要があるため）。
  - 本体: [`scripts/purge-snapshots.sh`](../../scripts/purge-snapshots.sh)
    （`paddock-analyze purge-snapshots --months <N>` を実行。保持月数は `PADDOCK_PURGE_MONTHS` で上書き可）
  - ログ先: `~/Library/Logs/paddock-backup.log`（backup-db と同じファイルに集約）
  - 保持月数の既定 6 ヶ月: #218（live オッズで α 再校正）が要する直近 3〜6 ヶ月の上端。CLI 既定の 12 は
    安全側の天井だが、6 ヶ月でも #218 の要件を満たしつつ定常ディスクを ≈5.4GB（12 ヶ月の約半分）に抑える。

## ⚠ スリープ取りこぼしと keep-awake の限界（#264）

`launchd` の `StartInterval` は **スリープ中は発火せず、スリープ解除もしない**。無人・離席で
画面が寝ると prefetch が発走直前 snapshot を取りこぼす（過去オッズは再取得不能で復元できない）。

- **keep-awake は best-effort**: `caffeinate -i` は**アイドルスリープのみ**抑止する。
  クラムシェル（蓋閉じ）スリープや `pmset` のスケジュールスリープは止められず（要 sudo/pmset）、
  **既にスリープ中の Mac を起こすこともできない**（朝に keep-awake が発火する時点で起きている必要）。
- **完全な堅牢化**は常時稼働ホスト（RasPi / 小型クラウド VM 等）へ prefetch を移設して
  ローカル Mac の電源・スリープ状態に依存させないこと（構成変更が大きいため別途）。
- **取りこぼしの事後検知**: 開催後に
  [`scripts/predict-check/snapshot_coverage.py`](../../scripts/predict-check/snapshot_coverage.py)
  で「最終 snapshot が発走の何分前で止まっているか」を一覧し、gap/none のレースを洗い出す。
  ```sh
  python3 scripts/predict-check/snapshot_coverage.py --date <YYYY-MM-DD>   # 既定 max-lag 10 分
  python3 scripts/predict-check/snapshot_coverage.py --date <YYYY-MM-DD> --fail-on-gap  # 監視用に exit 1
  ```

## 前提

- 当日の出馬表（`post_time` 入り）が朝の `paddock-fetch-card` 運用で DB に投入済みであること。
  未投入なら対象 0 件で no-op（安全）。
- release バイナリをビルド済みであること:
  `cargo build --release --bin paddock-fetch-card`

## macOS（launchd, 推奨）

```sh
# 有効化（prefetch / keep-awake / backup-db / backup-staleness / verify-backup-restore /
# purge-snapshots の 6 エージェントを配置して load。__REPO_ROOT__ は実パスに、常駐エージェントの
# __HOME__ はログ出力先へ置換される）
deployments/launchd/install.sh

# 状態確認 / ログ（launchd 経由は WORKDIR 固定）
launchctl list | grep com.paddock
tail -f /tmp/paddock-prefetch/logs/prefetch.log
tail -f /tmp/paddock-keep-awake/logs/keep-awake.log
# backup-db（毎日 23:30）/ backup-staleness（毎時 + 起動時）/ verify（日曜 04:00）/
# purge-snapshots（毎日 04:30）のログは同じファイルに集約
tail -f "$HOME/Library/Logs/paddock-backup.log"

# 無効化（prefetch / keep-awake のみ。backup-db / backup-staleness / verify-backup-restore /
# purge-snapshots は常駐で外れない）
deployments/launchd/uninstall.sh
```

`StartInterval=300`（5 分間隔）。prefetch/keep-awake は開催日だけ走らせたい場合、開催日朝に install、
夜に uninstall する運用でよい（常時 load でも対象 0 件なら no-op）。**backup-db・backup-staleness・
verify-backup-restore・purge-snapshots は常駐**で、`uninstall.sh` では外れない。個別に止めるときは手動で
`launchctl bootout gui/$UID/com.paddock.<label> && rm ~/Library/LaunchAgents/com.paddock.<label>.plist`
する（`<label>` = `backup-db` / `backup-staleness` / `verify-backup-restore` / `purge-snapshots`。
BACKUP.md のアンインストール手順と同一）。purge-snapshots の保持月数は `PADDOCK_PURGE_MONTHS`（既定 6）で
上書きできる。

## 手動・検証

```sh
# prefetch: DB の post_time で対象レースを確認（fetch しない・ネットワーク不要）
scripts/predict-check/prefetch_odds.sh --dry-run
scripts/predict-check/prefetch_odds.sh --at 15:10 --window-min 30 --dry-run

# prefetch: 1 回だけ実走（実際に netkeiba から取得し snapshots に積む）
scripts/predict-check/prefetch_odds.sh

# keep-awake: 当日の発走ウィンドウ算出を確認（caffeinate は起動しない）
scripts/predict-check/keep_awake.sh --dry-run
scripts/predict-check/keep_awake.sh --at 08:00 --dry-run    # 現在時刻を上書きして検証
```

## cron 代替（任意）

launchd を使わない場合の例（5 分間隔、開催日のみ等は運用で調整）:

```cron
*/5 * * * * cd /path/to/paddock && scripts/predict-check/prefetch_odds.sh >> /tmp/paddock-prefetch.cron.log 2>&1
```

多重起動は `prefetch_odds.sh` 側で抑止する（`flock` があれば flock、無ければ `mkdir` の
原子性によるフォールバックロック。素の macOS には flock が無いため後者で効かせる）。
launchd 経由は launchd 自身が同一 Label の重複起動をしないため二重ロック。
