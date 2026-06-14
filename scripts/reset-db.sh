#!/usr/bin/env bash
# 対象クローン/worktree の DB を空に戻す（再 seed / 再 ingest 前提）(#120)。
#
# paddock.db と WAL/SHM を退避（既定）または削除する。次回の app 起動 or seed-db.sh で
# スキーマごと再生成される。
#
# 使い方:
#   scripts/reset-db.sh                # ./data/paddock.db を .bak へ退避して空に戻す
#   scripts/reset-db.sh --to /other/data
#   scripts/reset-db.sh --no-backup    # 退避せず削除
set -euo pipefail

TO="data"
BACKUP=1

usage() {
    sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --to) TO="${2:?--to にパスが必要}"; shift 2 ;;
        --no-backup) BACKUP=0; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "不明な引数: $1" >&2; usage >&2; exit 2 ;;
    esac
done

DEST="$TO/paddock.db"
ts="$(date +%Y%m%d-%H%M%S)"
removed=0

for f in "$DEST" "$DEST-wal" "$DEST-shm"; do
    if [[ -e "$f" ]]; then
        if [[ "$BACKUP" -eq 1 ]]; then
            mv "$f" "$f.bak-$ts"
            echo "退避: $f -> $f.bak-$ts"
        else
            rm -f "$f"
            echo "削除: $f"
        fi
        removed=1
    fi
done

if [[ "$removed" -eq 0 ]]; then
    echo "reset 対象なし: $DEST は存在しない"
else
    echo "reset 完了: $DEST を空に戻した（再 seed / 再 ingest で再生成）"
fi
