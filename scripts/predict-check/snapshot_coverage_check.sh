#!/usr/bin/env bash
# 開催日の全レース発走後に snapshot 取りこぼしを自動検知する launchd ラッパ（#493）。
#
# prefetch（#237）は Mac スリープ・stale バイナリ・netkeiba 変化で毎サイクル失敗しても launchd は
# 正常発火し続けるため、発走直前オッズ snapshot（再取得不能資産）を丸ごと失っても事後まで気付けない。
# 本スクリプトは keep_awake.sh の END 判定と同じ基準（当日の最終 post_time + BUFFER_MIN）で
# 「全レース発走済み」を検出し、そのタイミングで一度だけ snapshot_coverage.py --fail-on-gap を
# 実行して gap/none/bad_ts が残るレースを洗い出し、あれば osascript で通知する。
#
# launchd から数分間隔で発火される前提（deployments/launchd/com.paddock.snapshot-coverage.plist）。
# 発走ウィンドウ中・開催外・未投入では no-op。最終発走後の初回だけ実走し、当日は marker で二度実行しない
# （毎サイクル通知の連投を防ぐ）。marker は永続パス（ログと同じ ~/Library/Logs 配下）に置く。
#
# 使い方:
#   scripts/predict-check/snapshot_coverage_check.sh [--date YYYY-MM-DD] [--buffer-min N] [--at HH:MM] [--force]
#   既定 DATE=今日(JST), BUFFER_MIN=10（keep_awake と対称）。--at は現在時刻の上書き（検証用）、
#   --force は marker と発走ウィンドウ判定を無視して即座に coverage を実行（手動検証用）。
#
# 環境変数:
#   PADDOCK_DB_URL             Postgres 接続 URL（既定 postgres://paddock:paddock@127.0.0.1:5432/paddock）
#   PADDOCK_COVERAGE_LOG       ログ出力先ファイル（既定 ~/Library/Logs/paddock-prefetch.log。prefetch と集約）
#   PADDOCK_COVERAGE_MAX_LAG   最終 snapshot が発走の何分前までを ok とするか（既定 10。snapshot_coverage 既定と対称）
set -euo pipefail

DATE=""
BUFFER_MIN="${BUFFER_MIN:-10}"
AT=""
FORCE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --date) DATE="${2:?--date には YYYY-MM-DD}"; shift 2 ;;
    --buffer-min) BUFFER_MIN="${2:?--buffer-min には分}"; shift 2 ;;
    --at) AT="${2:?--at には HH:MM}"; shift 2 ;;
    --force) FORCE=1; shift ;;
    -h|--help) sed -n '2,33p' "$0"; exit 0 ;;
    *) echo "不明な引数: $1" >&2; exit 2 ;;
  esac
done

DATE="${DATE:-$(TZ=Asia/Tokyo date +%Y-%m-%d)}"
[[ "$DATE" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || { echo "DATE は YYYY-MM-DD: $DATE" >&2; exit 2; }
[[ "$BUFFER_MIN" =~ ^[0-9]+$ ]] || { echo "BUFFER_MIN は整数（分）: $BUFFER_MIN" >&2; exit 2; }
if [ -n "$AT" ]; then
  [[ "$AT" =~ ^([0-9]{1,2}):([0-9]{2})$ ]] || { echo "--at は HH:MM: $AT" >&2; exit 2; }
  { [ "$((10#${BASH_REMATCH[1]}))" -le 23 ] && [ "$((10#${BASH_REMATCH[2]}))" -le 59 ]; } \
    || { echo "--at は 00:00〜23:59: $AT" >&2; exit 2; }
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DB_URL="${PADDOCK_DB_URL:-postgres://paddock:paddock@127.0.0.1:5432/paddock}"
MAX_LAG="${PADDOCK_COVERAGE_MAX_LAG:-10}"
LOG="${PADDOCK_COVERAGE_LOG:-$HOME/Library/Logs/paddock-prefetch.log}"
mkdir -p "$(dirname "$LOG")"
log() { echo "[$(date '+%Y-%m-%dT%H:%M:%S%z')] $*" | tee -a "$LOG"; }

notify() {
  # メッセージは argv 経由で AppleScript に渡す（" や \ で壊れない。backup-staleness と同方式）。
  osascript -e 'on run {msg}' -e 'display notification msg with title "paddock snapshot"' -e 'end run' -- "$1" >/dev/null 2>&1 || true
}

# 全レース発走後に一度だけ実行するための marker。当日分は再実行しない（通知連投を防ぐ）。
MARKER="$(dirname "$LOG")/.snapshot-coverage-done.$DATE"

run_coverage() {
  # snapshot_coverage.py --fail-on-gap は gap/none/bad_ts があれば exit 1。出力もログへ残す。
  local out rc
  out="$(PADDOCK_DB_URL="$DB_URL" python3 "$SCRIPT_DIR/snapshot_coverage.py" \
          --date "$DATE" --max-lag-min "$MAX_LAG" --fail-on-gap 2>&1)" && rc=0 || rc=$?
  printf '%s\n' "$out" | tee -a "$LOG" >/dev/null
  if [ "$rc" -ne 0 ]; then
    # 末尾の「要確認 NR: ...」行を通知に載せる（無ければ汎用文言）。
    local detail
    detail="$(printf '%s\n' "$out" | grep '要確認' | tail -1)"
    [ -n "$detail" ] || detail="snapshot 取りこぼし検知（${DATE}）。詳細はログ参照。"
    log "GAP: snapshot 取りこぼしあり（${DATE}）: $detail"
    notify "$detail"
  else
    log "OK: snapshot 全レース網羅（${DATE}・最終 snapshot 発走 ${MAX_LAG} 分前以内）"
  fi
}

if [ "$FORCE" -eq 1 ]; then
  log "[force] 発走ウィンドウ/marker を無視して coverage 実行（${DATE}）"
  run_coverage
  exit 0
fi

# 当日の最終 post_time（HH:MM）を DB から取得（keep_awake.sh と同一クエリ。文字列 MAX＝最終発走）。
# 接続不可は中断（無言 no-op にしない＝障害を取りこぼし扱いにしない）。
if ! LAST_POST="$(PGCONNECT_TIMEOUT="${PGCONNECT_TIMEOUT:-5}" psql "$DB_URL" -tA -c \
      "SELECT MAX(post_time) FROM race_cards \
       WHERE date='$DATE' AND post_time IS NOT NULL AND post_time ~ '^[0-9]{2}:[0-9]{2}\$';" 2>>"$LOG")"; then
  log "DB から最終 post_time を取得できず中断（接続不可等）"; exit 1
fi
LAST_POST="$(printf '%s' "$LAST_POST" | tr -d '[:space:]')"
if [ -z "$LAST_POST" ]; then
  log "対象なし: $DATE は post_time 入りカードが無い（開催外/未投入）。no-op"; exit 0
fi

# HH:MM → 分。END = 最終 post + BUFFER（keep_awake の END 判定と同基準）。
to_min() { local h="${1%%:*}" m="${1##*:}"; echo $((10#$h * 60 + 10#$m)); }
LAST_MIN="$(to_min "$LAST_POST")"
END_MIN=$((LAST_MIN + BUFFER_MIN))
if [ -n "$AT" ]; then NOW_MIN="$(to_min "$AT")"; else NOW_MIN="$(TZ=Asia/Tokyo date +'%H %M' | awk '{print $1*60+$2}')"; fi

if [ "$NOW_MIN" -lt "$END_MIN" ]; then
  log "発走ウィンドウ中: now=${NOW_MIN} < end=${END_MIN}（最終 post ${LAST_POST} + buffer ${BUFFER_MIN}分）。まだ検知しない"
  exit 0
fi

# 全レース発走後。当日分を既に実行済みなら二度実行しない。
if [ -f "$MARKER" ]; then
  log "当日分 coverage 実行済み（marker あり）: ${MARKER}。no-op"
  exit 0
fi

log "全レース発走後（now=${NOW_MIN} >= end=${END_MIN}）。snapshot 取りこぼし検知を実行（${DATE}）"
run_coverage
# 通知の有無に依らず当日分は実行済みとする（gap は再取得不能で連投しても改善しない）。
: > "$MARKER"
