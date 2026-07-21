# paddock DB バックアップ / 復元運用（#265）

`race_odds_snapshots`（発走直前オッズの時系列アーカイブ）は Colima の named volume
`paddock-pgdata` 1 か所にしか無く、過去オッズは**再取得不能**。volume 喪失（Colima reset /
`docker volume rm` / ディスク障害）に備え、full DB を定期退避する。

- **退避スクリプト**: [`scripts/backup-db.sh`](../../scripts/backup-db.sh)
- **日次スケジュール**: [`deployments/launchd/com.paddock.backup-db.plist`](../launchd/com.paddock.backup-db.plist)
- **退避先**:
  - **ローカル権威**（`PADDOCK_BACKUP_DIR`・既定 `~/paddock-backups`）: dump 本体。**世代管理（列挙→剪定）はここで行う**。launchd 下でも確実に列挙・削除でき、常に KEEP 世代に bounded。主脅威の Colima volume 喪失（reset / `docker volume rm`）はこのローカル退避だけで外れる。
  - **off-machine ミラー**（`PADDOCK_BACKUP_MIRROR_DIR`・**既定は空=無効**・オプトイン）: 指定すると各 dump をそこへコピーしディスク障害にも備える。**実ファイルシステム（外付け/NAS 等）を指定する**。iCloud Drive は使わない（下記）。
- **ミラー未設定時の警告（#507）**: 既定（ミラー無効）ではディスク障害でローカル権威も失うと復元不能になる。これに気づけるよう、`backup-db.sh` は未設定時に **ログへ毎回警告を残し**（`~/Library/Logs/paddock-backup.log`）、**macOS 通知は 7 日に 1 回**へ間引いて出す（間引き状態は `~/paddock-backups/.mirror-unset-warned` の mtime で管理。ミラー有効化で自動解除）。
- **形式 / 世代**: `paddock-YYYYMMDD-HHMMSS.dump`（`pg_dump -Fc` custom-format・圧縮込み）。既定で直近 14 世代を保持（`PADDOCK_BACKUP_KEEP`）。

> **iCloud をミラー先にしない理由（#494）**: launchd から実行すると **iCloud への "列挙" も "削除" も信頼
> できない**（書き込み=`cp` は効くが `ls`/glob は空を返し `rm` も反映されない macOS file-provider の癖・
> 検証で確認）。かつては iCloud Drive を既定ミラー先にしていたが、launchd 下では剪定が no-op になり dump
> が無制限に溜まる穴があった。ミラーは既定 off にし、必要なら剪定が確実に効く実ファイルシステムを指定する。

> **重要**: host の `pg_dump` が PG17 サーバより古い（v14 等）とダンプを拒否する。退避も復元も
> **必ず container 内（`paddock-postgres`・pg17）の pg_dump/pg_restore を `docker exec` で使う**
> （host に pg17 client を入れる必要はない）。

## 手動バックアップ

```sh
scripts/backup-db.sh
# 退避先/世代数を変える場合:
PADDOCK_BACKUP_DIR=/path/to/dir PADDOCK_BACKUP_KEEP=30 scripts/backup-db.sh
# off-machine ミラーを有効化する場合（実ファイルシステムを指定・iCloud は使わない）:
PADDOCK_BACKUP_MIRROR_DIR=/Volumes/ext/paddock-backups scripts/backup-db.sh
```

## launchd スケジュールのインストール

backup-db / backup-staleness / verify-backup-restore は prefetch / keep-awake と同じ `install.sh`
でまとめて配置する（#416 で二重規約を解消）。
`install.sh` が plist の `__REPO_ROOT__`（リポパス）と `__HOME__`（ログ出力先）を実値へ置換し load する。

```sh
deployments/launchd/install.sh                                      # 5 エージェントを配置
launchctl kickstart -k gui/$UID/com.paddock.backup-db               # backup-db 即時実行（動作確認）
launchctl kickstart -k gui/$UID/com.paddock.verify-backup-restore   # restore 検証 即時実行（動作確認）
tail -f ~/Library/Logs/paddock-backup.log                           # ログ確認（3エージェント集約）
```

スケジュール一覧:

| エージェント | スケジュール |
|---|---|
| `com.paddock.backup-db` | 毎日 23:30 |
| `com.paddock.backup-staleness` | 毎時 + 起動時 |
| `com.paddock.verify-backup-restore` | 毎週日曜 04:00（#474） |

> `kickstart` の 1 回実行で launchd の最小環境から docker まで到達できるか（PATH / docker context）を
> 必ず確認する。docker を `DOCKER_HOST` 環境変数で指している場合は launchd に引き継がれないため、
> plist の `EnvironmentVariables` に `DOCKER_HOST` を追記する（docker context 経由なら不要）。

アンインストール（backup-db / backup-staleness / verify-backup-restore は常駐のため `uninstall.sh`
では外れない。手動で bootout する）:

```sh
launchctl bootout gui/$UID/com.paddock.backup-db
rm ~/Library/LaunchAgents/com.paddock.backup-db.plist
# verify-backup-restore を止める場合:
launchctl bootout gui/$UID/com.paddock.verify-backup-restore
rm ~/Library/LaunchAgents/com.paddock.verify-backup-restore.plist
```

## 復元

> **前提**: 復元コマンドはすべて docker を使う。実行前に **colima（docker ランタイム）が起動していること**を
> 確認する。起動していない場合は `colima start`（または `brew services start colima`）を先に実行する。
> 詳細は [README「必要環境」の docker ランタイム項](../../README.md#必要環境) を参照。

### 全体復元（災害時・volume 喪失後）

新しい空の DB（マイグレーション前）へ dump を流し込む。`--clean --if-exists` で既存オブジェクトを
落としてから復元する（同名 DB へ上書き復元する場合）。

```sh
DUMP=~/paddock-backups/paddock-YYYYMMDD-HHMMSS.dump   # ミラーを有効化しているならミラー側のパスでも可
docker exec -i paddock-postgres pg_restore -U paddock -d paddock --clean --if-exists < "$DUMP"
```

> volume ごと失った場合は先に `docker compose -f deployments/compose.yaml up -d postgres` で空の
> paddock DB を作ってから上記を実行する（`-Fc` dump は全テーブル＋`_sqlx_migrations` を含むため、
> 復元後にアプリ起動しても再マイグレーションは走らない＝チェックサム一致）。

### snapshots だけ戻す（部分復元）

```sh
DUMP=~/paddock-backups/paddock-YYYYMMDD-HHMMSS.dump   # ミラーを有効化しているならミラー側のパスでも可
docker exec -i paddock-postgres pg_restore -U paddock -d paddock \
    --clean --if-exists -t race_odds_snapshots < "$DUMP"
```

> 部分復元は「スキーマ互換な live DB が既にある」前提。単表 `--clean` は FK/依存順の都合で
> 失敗しうる（そのときは全体復元を使う）。行データだけ差し戻すなら `--clean` を外し
> `--data-only` 単独で流す（重複を避けるなら事前に `TRUNCATE race_odds_snapshots`）。

## 復元検証（dump→restore の 1 サイクル・live DB を汚さない）

`scripts/verify-backup-restore.sh` が自動化している（`install.sh` で配置し毎週日曜 04:00 に実行・#474）。
手動実行は以下:

```sh
# 最新 dump を自動選択して検証（golden DB は read-only・scratch は使い捨て）
scripts/verify-backup-restore.sh

# 特定 dump を指定する場合
PADDOCK_VERIFY_DUMP=~/paddock-backups/paddock-YYYYMMDD-HHMMSS.dump \
    scripts/verify-backup-restore.sh

# ログ確認
tail -f ~/Library/Logs/paddock-backup.log
```

**突合テーブル**（既定: `race_odds_snapshots,races,horses`）:

- `race_odds_snapshots`: 再取得不能資産。行数不一致は致命的
- `races` / `horses`: スキーマ構造の sanity check

カスタマイズ: `PADDOCK_VERIFY_TABLES=race_odds_snapshots,race_odds scripts/verify-backup-restore.sh`

### 手動検証手順（スクリプト非使用の場合）

```sh
DUMP=~/paddock-backups/paddock-YYYYMMDD-HHMMSS.dump   # ミラーを有効化しているならミラー側のパスでも可
docker exec paddock-postgres createdb -U paddock paddock_restore_test
docker exec -i paddock-postgres pg_restore -U paddock -d paddock_restore_test < "$DUMP"
# 行数突合（source と一致すれば OK）
docker exec paddock-postgres psql -U paddock -d paddock_restore_test \
    -c "SELECT COUNT(*) FROM race_odds_snapshots;"
docker exec paddock-postgres psql -U paddock -d paddock \
    -c "SELECT COUNT(*) FROM race_odds_snapshots;"
docker exec paddock-postgres dropdb -U paddock paddock_restore_test
```

## スコープ外

- **capture 信頼性**（Mac スリープ・不在での取りこぼし）は別 issue。本運用は「蓄積済みデータの
  消失対策（退避と復元）」に限定する。
- launchd は Mac 起動時のみ動作（スリープ中は遅延実行 or skip）。日次で十分（取りこぼしても次回
  full dump で最新化される）。
