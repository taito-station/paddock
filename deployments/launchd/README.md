# 締切前 live オッズ自動 prefetch（#237）＋ keep-awake（#264）＋ 日次 DB バックアップ（#265）＋ バックアップ鮮度監視（#490）

4 つの launchd エージェントを `install.sh` でまとめて配置する:

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
# 有効化（prefetch / keep-awake / backup-db / backup-staleness の 4 エージェントを配置して load。
# __REPO_ROOT__ は実パスに、backup-db / backup-staleness の __HOME__ はログ出力先へ置換される）
deployments/launchd/install.sh

# 状態確認 / ログ（launchd 経由は WORKDIR 固定）
launchctl list | grep com.paddock
tail -f /tmp/paddock-prefetch/logs/prefetch.log
tail -f /tmp/paddock-keep-awake/logs/keep-awake.log
# backup-db（毎日 23:30）と backup-staleness（毎時 + 起動時）のログは同じファイルに集約
tail -f "$HOME/Library/Logs/paddock-backup.log"

# 無効化（prefetch / keep-awake のみ。backup-db / backup-staleness は常駐で外れない）
deployments/launchd/uninstall.sh
```

`StartInterval=300`（5 分間隔）。prefetch/keep-awake は開催日だけ走らせたい場合、開催日朝に install、
夜に uninstall する運用でよい（常時 load でも対象 0 件なら no-op）。**backup-db と backup-staleness は
常駐**で、`uninstall.sh` では外れない。backup-db を止めるときは手動で
`launchctl bootout gui/$UID/com.paddock.backup-db && rm ~/Library/LaunchAgents/com.paddock.backup-db.plist`、
backup-staleness を止めるときは同様に
`launchctl bootout gui/$UID/com.paddock.backup-staleness && rm ~/Library/LaunchAgents/com.paddock.backup-staleness.plist`
（BACKUP.md のアンインストール手順と同一）。

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
