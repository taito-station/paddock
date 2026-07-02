#!/usr/bin/env bash
# paddock DB（race_odds_snapshots 等の蓄積資産）を durable な場所へ退避する（#265）。
#
# race_odds_snapshots は Colima の named volume paddock-pgdata 1 か所にしか無く、過去オッズは
# 再取得不能。volume 喪失（Colima reset / docker volume rm / ディスク障害）に備え、full DB を
# custom-format（-Fc・圧縮込み）で dump し iCloud Drive 等へタイムスタンプ付きで退避＋世代管理する。
# 復元手順は deployments/db/BACKUP.md。日次実行は deployments/launchd/com.paddock.backup-db.plist。
#
# 重要: host の pg_dump が PG17 サーバより古い（v14 等）とダンプを拒否するため、**dump は
# container 内の pg_dump（バージョン一致）を docker exec で実行**する（host に pg17 client 不要）。
#
# 使い方:
#   scripts/backup-db.sh                 # 既定の退避先へ 1 回退避
#   PADDOCK_BACKUP_DIR=/path scripts/backup-db.sh
#   PADDOCK_BACKUP_KEEP=30 scripts/backup-db.sh
set -euo pipefail

usage() {
    cat <<'EOF'
backup-db.sh - paddock DB を durable な場所へ退避する（#265）

使い方:
  scripts/backup-db.sh            # $PADDOCK_BACKUP_DIR へ full DB dump を 1 回退避＋世代管理
  scripts/backup-db.sh -h|--help

環境変数:
  PADDOCK_BACKUP_DIR    退避先ディレクトリ
                        （既定: ~/Library/Mobile Documents/com~apple~CloudDocs/paddock-backups）
  PADDOCK_BACKUP_KEEP   保持する世代数（既定: 14。これを超えた古い dump を削除）
  PADDOCK_PG_CONTAINER  Postgres コンテナ名（既定: paddock-postgres）
  PADDOCK_PG_USER       DB ユーザ（既定: paddock）
  PADDOCK_PG_DB         DB 名（既定: paddock）
EOF
}

case "${1:-}" in
    -h|--help) usage; exit 0 ;;
    "") ;;
    *) echo "不明な引数: $1" >&2; usage >&2; exit 2 ;;
esac

BACKUP_DIR="${PADDOCK_BACKUP_DIR:-$HOME/Library/Mobile Documents/com~apple~CloudDocs/paddock-backups}"
KEEP="${PADDOCK_BACKUP_KEEP:-14}"
CONTAINER="${PADDOCK_PG_CONTAINER:-paddock-postgres}"
PG_USER="${PADDOCK_PG_USER:-paddock}"
PG_DB="${PADDOCK_PG_DB:-paddock}"

command -v docker >/dev/null || { echo "docker が見つからない（PATH を確認）" >&2; exit 1; }
if ! docker ps --format '{{.Names}}' | grep -qx "$CONTAINER"; then
    echo "コンテナ $CONTAINER が起動していない（docker compose -f deployments/compose.yaml up -d postgres）" >&2
    exit 1
fi

mkdir -p "$BACKUP_DIR"
ts="$(date +%Y%m%d-%H%M%S)"
final="$BACKUP_DIR/paddock-$ts.dump"
tmp="$final.part"
trap 'rm -f "$tmp"' EXIT

# container 内 pg_dump（バージョン一致）で full DB を custom-format 退避。stdout をホストファイルへ。
# 一時ファイル(.part)に書き、成功＋非空を確認してから最終名へ mv（中断で壊れた dump を残さない）。
if ! docker exec "$CONTAINER" pg_dump -U "$PG_USER" -d "$PG_DB" -Fc --no-owner --no-privileges > "$tmp"; then
    echo "pg_dump に失敗（container=$CONTAINER）" >&2
    exit 1
fi
if [[ ! -s "$tmp" ]]; then
    echo "dump が空（pg_dump は成功したが 0 バイト）" >&2
    exit 1
fi
mv "$tmp" "$final"
trap - EXIT

size="$(du -h "$final" | cut -f1)"
echo "退避完了: $final ($size)"

# 世代管理: 新しい順に KEEP 個を残し、それより古い paddock-*.dump を削除。
# macOS 既定の /bin/bash 3.2（launchd もこれを使う）に mapfile が無いため while-read で読む。
dumps=()
while IFS= read -r f; do
    [[ -n "$f" ]] && dumps+=("$f")
done < <(ls -1t "$BACKUP_DIR"/paddock-*.dump 2>/dev/null || true)
if (( ${#dumps[@]} > KEEP )); then
    i=0
    for old in "${dumps[@]}"; do
        if (( i >= KEEP )); then
            rm -f "$old"
            echo "古い世代を削除: $old"
        fi
        i=$((i + 1))
    done
fi
kept=$(( ${#dumps[@]} > KEEP ? KEEP : ${#dumps[@]} ))
echo "保持世代数: $kept / KEEP=$KEEP  退避先: $BACKUP_DIR"
