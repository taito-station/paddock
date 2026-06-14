#!/usr/bin/env bash
# 対象クローン/worktree の DB を空に戻す（再 seed / 再 ingest 前提）(#120)。
#
# paddock.db と WAL/SHM を退避（既定）または削除する。次回の app 起動 or seed-db.sh で
# スキーマごと再生成される。
#
# 前提: 対象クローンの app（predict / analyze / fetch 等）を停止してから実行すること。
# 稼働中プロセスが開いている DB の WAL/SHM を退避・削除すると整合性を壊しうる。
#
# 使い方:
#   scripts/reset-db.sh                # ./data/paddock.db を .bak へ退避して空に戻す
#   scripts/reset-db.sh --to /other/data
#   scripts/reset-db.sh --no-backup    # 退避せず削除
#   scripts/reset-db.sh --force        # primary clone の data でも実行する
set -euo pipefail

TO="data"
BACKUP=1
FORCE=0

usage() {
    cat <<'EOF'
reset-db.sh - 対象クローンの DB を空に戻す（再 seed / 再 ingest 前提）(#120)

paddock.db と WAL/SHM を退避（既定）または削除する。次回の app 起動 or seed-db.sh で
スキーマごと再生成される。

使い方:
  scripts/reset-db.sh                # ./data/paddock.db を .bak へ退避して空に戻す
  scripts/reset-db.sh --to /other/data
  scripts/reset-db.sh --no-backup    # 退避せず削除
  scripts/reset-db.sh --force        # primary clone の data でも実行する

オプション:
  --to <dir>     対象 data ディレクトリ（既定: data）
  --no-backup    .bak へ退避せず削除する
  --force        primary clone の data（golden 元）への reset を許可する
  -h, --help     このヘルプを表示
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --to) TO="${2:?--to にパスが必要}"; shift 2 ;;
        --no-backup) BACKUP=0; shift ;;
        --force) FORCE=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "不明な引数: $1" >&2; usage >&2; exit 2 ;;
    esac
done

TO="${TO%/}"          # 末尾スラッシュを正規化（data/ → data）
[[ -n "$TO" ]] || TO="."
DEST="$TO/paddock.db"

# primary clone の data を誤って reset すると全クローンの seed 元（golden）を失う。
# seed-db.sh が `-ef` で primary への書き込みを防ぐのと対称に、reset でも git で primary を
# 検出できる場合は対象が primary の data なら既定で中断する（--force で明示的に上書き）。
if [[ "$FORCE" -ne 1 ]]; then
    common_dir="$(git rev-parse --git-common-dir 2>/dev/null || true)"
    if [[ -n "$common_dir" ]]; then
        primary_data="$(cd "$(dirname "$common_dir")" && pwd)/data"
        target_data="$(cd "$TO" 2>/dev/null && pwd || true)"
        if [[ -n "$target_data" && "$target_data" == "$primary_data" ]]; then
            echo "対象が primary clone の data（golden 元）です: ${target_data}。" >&2
            echo "全クローンの seed 元を失うため既定では中断する。意図的なら --force を付ける" >&2
            exit 1
        fi
    fi
fi
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
