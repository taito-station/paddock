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
#
# 注: mtime 取得に `stat -f`（BSD/macOS の書式）を使う。本スクリプトは launchd 配下＝macOS 専用
# 運用のため移植性は考慮しない（Linux の GNU stat では `-f` はファイルシステム情報になり動作が異なる）。
set -euo pipefail

log() { echo "[$(date '+%Y-%m-%dT%H:%M:%S%z')] $*"; }

notify() {
    # メッセージは argv 経由で AppleScript に渡す（パス/ファイル名の " や \ で文字列が壊れないように）。
    osascript -e 'on run {msg}' -e 'display notification msg with title "paddock backup"' -e 'end run' -- "$1" >/dev/null 2>&1 || true
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
    # age_hours は切り捨て表示のため境界では閾値と同値に見える。判定は秒単位（age_secs>閾値）で厳密。
    msg="最新 dump が鮮度閾値 ${STALE_THRESHOLD_HOURS}h を超過（約 ${age_hours}h 経過）: $basename_latest"
    log "STALE: $msg"
    notify "$msg"
else
    log "OK: 最新 dump は ${age_hours}h 前（閾値 ${STALE_THRESHOLD_HOURS}h 以内）: $(basename "$latest")"
fi
