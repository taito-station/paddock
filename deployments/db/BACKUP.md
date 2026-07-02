# paddock DB バックアップ / 復元運用（#265）

`race_odds_snapshots`（発走直前オッズの時系列アーカイブ）は Colima の named volume
`paddock-pgdata` 1 か所にしか無く、過去オッズは**再取得不能**。volume 喪失（Colima reset /
`docker volume rm` / ディスク障害）に備え、full DB を定期退避する。

- **退避スクリプト**: [`scripts/backup-db.sh`](../../scripts/backup-db.sh)
- **日次スケジュール**: [`deployments/launchd/com.paddock.backup-db.plist`](../launchd/com.paddock.backup-db.plist)
- **退避先（既定）**: `~/Library/Mobile Documents/com~apple~CloudDocs/paddock-backups`（iCloud Drive＝off-machine で durable）。`PADDOCK_BACKUP_DIR` で変更可。
- **形式 / 世代**: `paddock-YYYYMMDD-HHMMSS.dump`（`pg_dump -Fc` custom-format・圧縮込み）。既定で直近 14 世代を保持（`PADDOCK_BACKUP_KEEP`）。

> **重要**: host の `pg_dump` が PG17 サーバより古い（v14 等）とダンプを拒否する。退避も復元も
> **必ず container 内（`paddock-postgres`・pg17）の pg_dump/pg_restore を `docker exec` で使う**
> （host に pg17 client を入れる必要はない）。

## 手動バックアップ

```sh
scripts/backup-db.sh
# 退避先/世代数を変える場合:
PADDOCK_BACKUP_DIR=/path/to/dir PADDOCK_BACKUP_KEEP=30 scripts/backup-db.sh
```

## 日次スケジュール（launchd）のインストール

```sh
# __PADDOCK_REPO__ と __HOME__ を実値へ置換して LaunchAgents へ配置
sed -e "s#__PADDOCK_REPO__#$PWD#g" -e "s#__HOME__#$HOME#g" \
    deployments/launchd/com.paddock.backup-db.plist \
    > ~/Library/LaunchAgents/com.paddock.backup-db.plist

launchctl load ~/Library/LaunchAgents/com.paddock.backup-db.plist   # 有効化（毎日 23:30）
launchctl kickstart -k gui/$UID/com.paddock.backup-db               # 即時実行（動作確認）
tail -f ~/Library/Logs/paddock-backup.log                           # ログ確認
```

アンインストール:

```sh
launchctl bootout gui/$UID/com.paddock.backup-db
rm ~/Library/LaunchAgents/com.paddock.backup-db.plist
```

## 復元

### 全体復元（災害時・volume 喪失後）

新しい空の DB（マイグレーション前）へ dump を流し込む。`--clean --if-exists` で既存オブジェクトを
落としてから復元する（同名 DB へ上書き復元する場合）。

```sh
DUMP=~/Library/Mobile\ Documents/com~apple~CloudDocs/paddock-backups/paddock-YYYYMMDD-HHMMSS.dump
docker exec -i paddock-postgres pg_restore -U paddock -d paddock --clean --if-exists < "$DUMP"
```

> volume ごと失った場合は先に `docker compose -f deployments/compose.yaml up -d postgres` で空の
> paddock DB を作ってから上記を実行する（`-Fc` dump は全テーブル＋`_sqlx_migrations` を含むため、
> 復元後にアプリ起動しても再マイグレーションは走らない＝チェックサム一致）。

### snapshots だけ戻す（部分復元）

```sh
docker exec -i paddock-postgres pg_restore -U paddock -d paddock \
    --clean --if-exists -t race_odds_snapshots < "$DUMP"
```

## 復元検証（dump→restore の 1 サイクル・live DB を汚さない）

scratch DB へ復元して行数が一致するか確認する。

```sh
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
