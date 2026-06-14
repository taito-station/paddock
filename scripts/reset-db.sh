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
    cat <<'EOF'
reset-db.sh - 対象クローンの DB を空に戻す（再 seed / 再 ingest 前提）(#120)

paddock.db と WAL/SHM を退避（既定）または削除する。次回の app 起動 or seed-db.sh で
スキーマごと再生成される。

使い方:
  scripts/reset-db.sh                # ./data/paddock.db を .bak へ退避して空に戻す
  scripts/reset-db.sh --to /other/data
  scripts/reset-db.sh --no-backup    # 退避せず削除

オプション:
  --to <dir>     対象 data ディレクトリ（既定: data）
  --no-backup    .bak へ退避せず削除する
  -h, --help     このヘルプを表示
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --to) TO="${2:?--to にパスが必要}"; shift 2 ;;
        --no-backup) BACKUP=0; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "不明な引数: $1" >&2; usage >&2; exit 2 ;;
    esac
done

TO="${TO%/}"          # 末尾スラッシュを正規化（data/ → data）
[[ -n "$TO" ]] || TO="."
DEST="$TO/paddock.db"
# ts に PID を付けて、同一秒に別プロセスが退避しても .bak が衝突しないようにする。
ts="$(date +%Y%m%d-%H%M%S)-$$"
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
