# paddock DB バックアップ / 復元運用（#265）

`race_odds_snapshots`（発走直前オッズの時系列アーカイブ）は Postgres コンテナの named volume
`paddock-pgdata` 1 か所にしか無く、過去オッズは**再取得不能**。volume 喪失（VM 削除 /
`nerdctl volume rm` or `docker volume rm` / ディスク障害）に備え、full DB を定期退避する。

## 実行環境（lima/nerdctl or colima/docker）の自動判定（#731）

Postgres コンテナは開発機によって **Lima VM 内の rootless nerdctl** または **colima（docker）**
のいずれかで動く。`scripts/backup-db.sh` / `scripts/verify-backup-restore.sh` は
[`scripts/lib/pg-container.sh`](../../scripts/lib/pg-container.sh) を source し、`PADDOCK_PG_RUNTIME`
（既定 `auto`）で実行環境を決める:

- `auto`（既定）: `limactl` があり VM（`PADDOCK_LIMA_VM`・既定 `paddock`）が `Running` なら **lima**。
  そうでなく `docker` があり対象コンテナが `docker ps` に見えれば **docker**。どちらにも当たらなければ
  判定に使った事実（limactl の有無・VM の状態・docker の応答）を列挙して失敗する（黙って進めない）。
- `lima` / `docker`: 判定を固定したい場合に明示指定する。`lima` 明示でも VM の状態は確認し、
  `Running` でなければ「`limactl start <VM>` で起動」と案内して失敗する。
- exec は環境に応じて `limactl shell "$PADDOCK_LIMA_VM" -- nerdctl exec ...`（lima）または
  `docker exec ...`（docker）を使う。
- **判定の非対称は意図的**: lima は「VM が `Running`」で確定し、そこでコンテナが見えなくても docker へは
  流さない（失敗させる）。docker 側（colima 等）には移行前の旧 DB が残っていることがあり、見えた方へ
  黙って流れると別インスタンスの DB を退避・検証してしまうため。
- **どこから取ったかを必ず残す**: 両スクリプトは毎回 `コンテナ実行環境: runtime=... vm=... container=...`
  をログに出す。`backup-db.sh` は dump と対の `<dump>.runtime` サイドカーにも同じ 1 行を書く。
  auto 判定が「lima VM が停止しているため docker 側を使った」場合は、旧 DB を退避・検証している
  可能性があるので **警告ログと macOS 通知**を出す（#731。移行前の旧 DB で世代が置き換わるのを防ぐ）。

```sh
PADDOCK_PG_RUNTIME=lima scripts/backup-db.sh            # lima を強制
PADDOCK_PG_RUNTIME=docker scripts/backup-db.sh           # docker を強制
PADDOCK_LIMA_VM=paddock-dev scripts/backup-db.sh         # 別名 VM を使う場合
```

- **退避スクリプト**: [`scripts/backup-db.sh`](../../scripts/backup-db.sh)
- **日次スケジュール**: [`deployments/launchd/com.paddock.backup-db.plist`](../launchd/com.paddock.backup-db.plist)
- **退避先**:
  - **ローカル権威**（`PADDOCK_BACKUP_DIR`・既定 `~/paddock-backups`）: dump 本体。**世代管理（列挙→剪定）はここで行う**。launchd 下でも確実に列挙・削除でき、常に KEEP 世代に bounded。主脅威のコンテナ volume 喪失（Lima VM 削除・`nerdctl volume rm` / colima reset・`docker volume rm`）はこのローカル退避だけで外れる。
  - **off-machine ミラー**（`PADDOCK_BACKUP_MIRROR_DIR`・**既定は空=無効**・オプトイン）: 指定すると各 dump をそこへコピーしディスク障害にも備える。**実ファイルシステム（外付け/NAS 等）を指定する**。iCloud Drive は使わない（下記）。
- **ミラー未設定時の警告（#507）**: 既定（ミラー無効）ではディスク障害でローカル権威も失うと復元不能になる。これに気づけるよう、`backup-db.sh` は未設定時に **ログへ毎回警告を残し**（`~/Library/Logs/paddock-backup.log`）、**macOS 通知は 7 日に 1 回**へ間引いて出す（間引き状態は `~/paddock-backups/.mirror-unset-warned` の mtime で管理。ミラー有効化で自動解除）。
- **形式 / 世代**: `paddock-YYYYMMDD-HHMMSS.dump`（`pg_dump -Fc` custom-format・圧縮込み）。既定で直近 14 世代を保持（`PADDOCK_BACKUP_KEEP`）。

> **iCloud をミラー先にしない理由（#494）**: launchd から実行すると **iCloud への "列挙" も "削除" も信頼
> できない**（書き込み=`cp` は効くが `ls`/glob は空を返し `rm` も反映されない macOS file-provider の癖・
> 検証で確認）。かつては iCloud Drive を既定ミラー先にしていたが、launchd 下では剪定が no-op になり dump
> が無制限に溜まる穴があった。ミラーは既定 off にし、必要なら剪定が確実に効く実ファイルシステムを指定する。

> **重要**: host の `pg_dump` が PG17 サーバより古い（v14 等）とダンプを拒否する。退避も復元も
> **必ず container 内（`paddock-postgres`・pg17）の pg_dump/pg_restore を、実行環境に応じた exec
> （`limactl shell ... -- nerdctl exec` または `docker exec`）で使う**（host に pg17 client を
> 入れる必要はない。実行環境の判定は上記「実行環境の自動判定」節を参照）。

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

> `kickstart` の 1 回実行で launchd の最小環境からコンテナ実行環境（limactl または docker）まで
> 到達できるか（PATH / docker context）を必ず確認する。docker を `DOCKER_HOST` 環境変数で指している
> 場合は launchd に引き継がれないため、plist の `EnvironmentVariables` に `DOCKER_HOST` を追記する
> （docker context 経由なら不要）。

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

> **前提**: 以下の手動コマンドは、現行の実行環境である **Lima VM 内の nerdctl**（VM 名は
> `PADDOCK_LIMA_VM` の既定 `paddock`）で書き、docker（colima 等）の場合をコメントで併記する。
> スクリプト（`backup-db.sh` / `verify-backup-restore.sh`）は実行環境を自動で判定するが、
> **手動コマンドは自動判定の対象外**なので、どちらで動いているかは実行者が確かめる
> （lima: `limactl list` で `Running`、必要なら `limactl start paddock`。docker: `colima start`）。
> 復元に使う dump がどの実行環境から取られたかは、dump と対の `<dump>.runtime` サイドカーで確認できる
> （サイドカーの無い dump は #731 より前のもの）。docker 側の詳細は
> [README「必要環境」の docker ランタイム項](../../README.md#必要環境) を参照。

### 全体復元（災害時・volume 喪失後）

新しい空の DB（マイグレーション前）へ dump を流し込む。`--clean --if-exists` で既存オブジェクトを
落としてから復元する（同名 DB へ上書き復元する場合）。

```sh
DUMP=~/paddock-backups/paddock-YYYYMMDD-HHMMSS.dump   # ミラーを有効化しているならミラー側のパスでも可
limactl shell paddock -- nerdctl exec -i paddock-postgres pg_restore -U paddock -d paddock --clean --if-exists < "$DUMP"
# docker（colima 等）の場合:
# docker exec -i paddock-postgres pg_restore -U paddock -d paddock --clean --if-exists < "$DUMP"
```

> volume ごと失った場合は先に `limactl shell paddock -- nerdctl compose -f deployments/compose.yaml up -d postgres`
> （docker: `docker compose -f deployments/compose.yaml up -d postgres`）
> で空の paddock DB を作ってから上記を実行する（`-Fc` dump は全テーブル＋`_sqlx_migrations` を
> 含むため、復元後にアプリ起動しても再マイグレーションは走らない＝チェックサム一致）。

### snapshots だけ戻す（部分復元）

```sh
DUMP=~/paddock-backups/paddock-YYYYMMDD-HHMMSS.dump   # ミラーを有効化しているならミラー側のパスでも可
limactl shell paddock -- nerdctl exec -i paddock-postgres pg_restore -U paddock -d paddock \
    --clean --if-exists -t race_odds_snapshots < "$DUMP"
# docker の場合は `limactl shell paddock -- nerdctl exec` を `docker exec` に読み替える
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

**突合の基準は live golden ではなくサイドカー**（`<dump>.rowcounts`）。`backup-db.sh` が dump 生成と
ほぼ同時刻の各テーブル `COUNT(*)` を `paddock-YYYYMMDD-HHMMSS.dump.rowcounts` に記録し、検証側は
その記録値と scratch 復元行数を厳密比較する。live golden と比べると、検証（日 04:00）が dump 生成
（土 23:30）から数時間ズレる間に golden へ INSERT が入り「scratch < golden」で**偽 FAIL** する。
サイドカー方式なら時刻ズレが原理的に無く、行の増加も欠落も正しく判定できる（race-free）。
サイドカーが無い旧 dump は行数突合を skip（構造健全性は `backup-db.sh` の `pg_restore --list` で担保）。

突合テーブルのカスタマイズは **`backup-db.sh` 側の** `PADDOCK_VERIFY_TABLES`（サイドカー生成時に反映）:
`PADDOCK_VERIFY_TABLES=race_odds_snapshots,race_odds scripts/backup-db.sh`

### 手動検証手順（スクリプト非使用の場合）

```sh
DUMP=~/paddock-backups/paddock-YYYYMMDD-HHMMSS.dump   # ミラーを有効化しているならミラー側のパスでも可
PG="limactl shell paddock -- nerdctl exec"   # docker の場合は PG="docker exec"
$PG paddock-postgres createdb -U paddock paddock_restore_test
$PG -i paddock-postgres pg_restore -U paddock -d paddock_restore_test < "$DUMP"
# 行数突合（source と一致すれば OK）
$PG paddock-postgres psql -U paddock -d paddock_restore_test \
    -c "SELECT COUNT(*) FROM race_odds_snapshots;"
$PG paddock-postgres psql -U paddock -d paddock \
    -c "SELECT COUNT(*) FROM race_odds_snapshots;"
$PG paddock-postgres dropdb -U paddock paddock_restore_test
```

## スコープ外

- **capture 信頼性**（Mac スリープ・不在での取りこぼし）は別 issue。本運用は「蓄積済みデータの
  消失対策（退避と復元）」に限定する。
- launchd は Mac 起動時のみ動作（スリープ中は遅延実行 or skip）。日次で十分（取りこぼしても次回
  full dump で最新化される）。
