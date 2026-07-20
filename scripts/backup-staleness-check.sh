#!/usr/bin/env bash
# paddock DB バックアップの鮮度を確認し、最新 dump が古すぎる場合に通知する（#490）。
#
# backup-db.sh は launchd（毎日 23:30）から実行されるが、Mac スリープや colima 停止により
# 無言で欠落することがある（実測: 直近 9 日で 2 日分欠落）。失敗通知だけでは「実行されなかった」
# 欠落を拾えないため、本スクリプトが鮮度監視を担う。
# launchd（毎時 + RunAtLoad=true）から定期発火し、スリープ復帰時に catch-up 検知する。
#
# 使い方:
#   scripts/backup-staleness-check.sh
#   STALE_THRESHOLD_HOURS=48 scripts/backup-staleness-check.sh
#
# 環境変数:
#   PADDOCK_BACKUP_DIR      バックアップの権威ディレクトリ（既定: ~/paddock-backups）
#   STALE_THRESHOLD_HOURS   この時間（時間単位）を超えた最新 dump を古いとみなす（既定: 36）
set -euo pipefail

log() { echo "[$(date '+%Y-%m-%dT%H:%M:%S%z')] $*"; }

notify() {
    osascript -e "display notification \"$1\" with title \"paddock backup\"" >/dev/null 2>&1 || true
}

BACKUP_DIR="${PADDOCK_BACKUP_DIR:-$HOME/paddock-backups}"
STALE_THRESHOLD_HOURS="${STALE_THRESHOLD_HOURS:-36}"
STALE_THRESHOLD_SECS=$(( STALE_THRESHOLD_HOURS * 3600 ))

# ディレクトリ非存在チェック
if [ ! -d "$BACKUP_DIR" ]; then
    msg="バックアップディレクトリが存在しない: $BACKUP_DIR"
    log "STALE: $msg"
    notify "$msg"
    exit 0
fi

# 最新 dump を探す（paddock-*.dump）
latest=""
latest_mtime=0
while IFS= read -r f; do
    [ -n "$f" ] || continue
    mtime="$(stat -f '%m' "$f" 2>/dev/null)" || continue
    if [ "$mtime" -gt "$latest_mtime" ]; then
        latest_mtime="$mtime"
        latest="$f"
    fi
done < <(find "$BACKUP_DIR" -maxdepth 1 -type f -name 'paddock-*.dump' 2>/dev/null)

# dump ゼロ件チェック
if [ -z "$latest" ]; then
    msg="バックアップ dump が存在しない: $BACKUP_DIR"
    log "STALE: $msg"
    notify "$msg"
    exit 0
fi

# 鮮度チェック
now="$(date +%s)"
age_secs=$(( now - latest_mtime ))
age_hours=$(( age_secs / 3600 ))

if [ "$age_secs" -gt "$STALE_THRESHOLD_SECS" ]; then
    basename_latest="$(basename "$latest")"
    msg="最新 dump が ${age_hours}h 前: $basename_latest (閾値 ${STALE_THRESHOLD_HOURS}h)"
    log "STALE: $msg"
    notify "$msg"
else
    log "OK: 最新 dump は ${age_hours}h 前（閾値 ${STALE_THRESHOLD_HOURS}h 以内）: $(basename "$latest")"
fi
