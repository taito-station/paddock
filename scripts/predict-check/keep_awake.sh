#!/usr/bin/env bash
# 開催日の発走ウィンドウ中、Mac のアイドルスリープを抑止して締切前 prefetch（#237）の
# launchd タイマーを確実に発火させる（#264）。
#
# launchd の StartInterval はスリープ中に発火しないため、無人・離席で画面が寝ると prefetch が
# 取りこぼす（発走直前 snapshot が欠落＝過去オッズ再取得不能）。本スクリプトは当日の最終 post_time
# まで `caffeinate -i` でアイドルスリープを抑止し、prefetch の 5 分タイマーを回し続ける。
#
# **限界（best-effort）**: caffeinate はアイドルスリープを止めるだけで、
#   - クラムシェル（蓋閉じ）スリープや `pmset` のスケジュールスリープは止められない（要 sudo/pmset）
#   - 既にスリープ中の Mac を起こすことはできない（朝にこのジョブが発火する時点で起きている必要がある）
# 完全な堅牢化は常時稼働ホスト（RasPi/小型 VM 等）への prefetch 移設（deployments/launchd/README）。
#
# 使い方:
#   scripts/predict-check/keep_awake.sh [--date YYYY-MM-DD] [--buffer-min N] [--at HH:MM] [--dry-run]
#   既定 DATE=今日(JST), BUFFER_MIN=10。--at は現在時刻の上書き（検証用）、--dry-run は計算のみ。
#
# 環境変数:
#   PADDOCK_DB_URL  Postgres 接続 URL（既定 postgres://paddock:paddock@127.0.0.1:5432/paddock）
#   WORKDIR         ログ出力先（既定 $TMPDIR/paddock-keep-awake）
set -euo pipefail

DATE=""
BUFFER_MIN="${BUFFER_MIN:-10}"
AT=""
DRY_RUN=0
while [ $# -gt 0 ]; do
  case "$1" in
    --date) DATE="${2:?--date には YYYY-MM-DD}"; shift 2 ;;
    --buffer-min) BUFFER_MIN="${2:?--buffer-min には分}"; shift 2 ;;
    --at) AT="${2:?--at には HH:MM}"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "不明な引数: $1" >&2; exit 2 ;;
  esac
done

DATE="${DATE:-$(TZ=Asia/Tokyo date +%Y-%m-%d)}"
[[ "$DATE" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || { echo "DATE は YYYY-MM-DD: $DATE" >&2; exit 2; }
[[ "$BUFFER_MIN" =~ ^[0-9]+$ ]] || { echo "BUFFER_MIN は整数（分）: $BUFFER_MIN" >&2; exit 2; }
[ -z "$AT" ] || [[ "$AT" =~ ^[0-9]{1,2}:[0-9]{2}$ ]] || { echo "--at は HH:MM: $AT" >&2; exit 2; }

DB_URL="${PADDOCK_DB_URL:-postgres://paddock:paddock@127.0.0.1:5432/paddock}"
WORKDIR="${WORKDIR:-${TMPDIR:-/tmp}/paddock-keep-awake}"
mkdir -p "$WORKDIR/logs"
LOG="$WORKDIR/logs/keep-awake.log"
log() { echo "[$(date '+%Y-%m-%dT%H:%M:%S%z')] $*" | tee -a "$LOG"; }

# 当日の最終 post_time（HH:MM）を DB から取得。post_time は TEXT 'HH:MM'（ゼロ埋め）なので
# 文字列 MAX で時刻最大＝最終発走になる。post_time NULL は除外。接続不可は中断（無言で
# no-op にしない＝障害を取りこぼし扱いにしない）。
if ! LAST_POST="$(PGCONNECT_TIMEOUT="${PGCONNECT_TIMEOUT:-5}" psql "$DB_URL" -tA -c \
      "SELECT MAX(post_time) FROM race_cards \
       WHERE date='$DATE' AND post_time IS NOT NULL AND post_time ~ '^[0-9]{2}:[0-9]{2}\$';" 2>>"$LOG")"; then
  log "DB から最終 post_time を取得できず中断（接続不可等）"; exit 1
fi
LAST_POST="$(printf '%s' "$LAST_POST" | tr -d '[:space:]')"
if [ -z "$LAST_POST" ]; then
  log "対象なし: $DATE は post_time 入りカードが無い（開催外/未投入）。no-op"; exit 0
fi

# HH:MM → 分。now は --at 優先、無ければ JST 現在時刻。
to_min() { local h="${1%%:*}" m="${1##*:}"; echo $((10#$h * 60 + 10#$m)); }
LAST_MIN="$(to_min "$LAST_POST")"
END_MIN=$((LAST_MIN + BUFFER_MIN))
if [ -n "$AT" ]; then NOW_MIN="$(to_min "$AT")"; else NOW_MIN="$(TZ=Asia/Tokyo date +'%H %M' | awk '{print $1*60+$2}')"; fi

if [ "$NOW_MIN" -ge "$END_MIN" ]; then
  log "発走ウィンドウ終了済み: now=${NOW_MIN} >= end=${END_MIN}（最終 post ${LAST_POST} + buffer ${BUFFER_MIN}分）。no-op"
  exit 0
fi
SECS=$(((END_MIN - NOW_MIN) * 60))

if [ "$DRY_RUN" -eq 1 ]; then
  log "[dry-run] $DATE 最終post=$LAST_POST end=$END_MIN(now=$NOW_MIN) → caffeinate -i -t ${SECS}s"
  exit 0
fi

# caffeinate は macOS 専用。非 macOS や不在環境では何もしない（移設先ホスト等で誤動作させない）。
if ! command -v caffeinate >/dev/null 2>&1; then
  log "caffeinate 不在（非 macOS?）。アイドルスリープ抑止はスキップ"; exit 0
fi

# 多重起動防止: lockdir に稼働中 caffeinate の PID を記録し、生きていれば再起動しない
# （StartInterval で 5 分毎に発火しても caffeinate を積み上げない）。caffeinate は -t で自動終了し
# PID が死ぬと lock は stale 化するので、次回起動時に PID 生存を見て掃除する（self-heal。
# 専用の後始末プロセスは持たない＝兄弟 PID を wait できない罠を避ける）。
LOCK_DIR="/tmp/paddock-keep-awake.lock.d"
if [ -d "$LOCK_DIR" ] && [ -f "$LOCK_DIR/pid" ] \
   && kill -0 "$(cat "$LOCK_DIR/pid" 2>/dev/null)" 2>/dev/null; then
  log "既に caffeinate 稼働中（pid $(cat "$LOCK_DIR/pid")）。重複起動せず終了"; exit 0
fi
rm -rf "$LOCK_DIR" 2>/dev/null || true
mkdir "$LOCK_DIR" 2>/dev/null || { log "lock 取得失敗（競合）。終了"; exit 0; }

# アイドルスリープを END まで抑止。-t で自動終了するので開放忘れが無い。nohup+disown で launchd
# ジョブ本体終了後も caffeinate を存続させ、SIGHUP で巻き添え終了しないようにする。
nohup caffeinate -i -t "$SECS" >/dev/null 2>&1 &
CAF_PID=$!
echo "$CAF_PID" > "$LOCK_DIR/pid"
disown 2>/dev/null || true
log "caffeinate -i -t ${SECS}s 起動（pid ${CAF_PID}）。${DATE} 最終post ${LAST_POST} まで抑止"
