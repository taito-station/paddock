#!/usr/bin/env bash
# 締切前 live オッズの自動 prefetch — 発走 N 分以内のレースの最新オッズを取得し、
# race_odds_snapshots（#232）に締切前 live スナップショットを蓄積する（#237）。
#
# refresh_ev.sh（EV 算出まで行う当日監視ツール）とは別物で、本スクリプトは odds 取得だけに
# 特化する。レース選択は #235 の DB post_time（race_cards.post_time）で行い、netkeiba を
# 都度スクレイプしない。launchd 等から数分間隔で起動される前提（deployments/launchd/）。
#
# 使い方:
#   scripts/predict-check/prefetch_odds.sh [--date YYYY-MM-DD] [--window-min N] [--at HH:MM] [--dry-run]
#   既定 DATE=今日(JST), WINDOW_MIN=30。--dry-run は対象レースの表示のみで fetch しない。
#
# 環境変数:
#   PADDOCK_DB_URL  Postgres 接続 URL（既定: postgres://paddock:paddock@127.0.0.1:5432/paddock）
#                   host は 127.0.0.1 を使う（#212, localhost の ::1 先解決で別 postgres 事故回避）。
#   WORKDIR         ログ出力先（既定: $TMPDIR/paddock-prefetch）
#   WINDOW_MIN      発走まで何分以内を対象にするか（既定 30。引数 --window-min が優先）
#
# 前提: その日の出馬表（post_time 入り）は朝の paddock-fetch-card 運用で投入済みであること。
# 未投入なら対象 0 件で no-op（正常終了）。
set -euo pipefail

DATE=""
WINDOW_MIN="${WINDOW_MIN:-30}"
AT=""
DRY_RUN=0
while [ $# -gt 0 ]; do
  case "$1" in
    --date) DATE="${2:?--date には YYYY-MM-DD}"; shift 2 ;;
    --window-min) WINDOW_MIN="${2:?--window-min には分}"; shift 2 ;;
    --at) AT="${2:?--at には HH:MM}"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "不明な引数: $1" >&2; exit 2 ;;
  esac
done

# 既定日付は JST の今日（launchd/cron の TZ に依存しないよう明示）。
DATE="${DATE:-$(TZ=Asia/Tokyo date +%Y-%m-%d)}"
[[ "$DATE" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || { echo "DATE は YYYY-MM-DD: $DATE" >&2; exit 2; }
[[ "$WINDOW_MIN" =~ ^[0-9]+$ ]] || { echo "WINDOW_MIN は整数（分）: $WINDOW_MIN" >&2; exit 2; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DB_URL="${PADDOCK_DB_URL:-postgres://paddock:paddock@127.0.0.1:5432/paddock}"
WORKDIR="${WORKDIR:-${TMPDIR:-/tmp}/paddock-prefetch}"
mkdir -p "$WORKDIR/logs"
LOG="$WORKDIR/logs/prefetch.log"

log() { echo "[$(date '+%Y-%m-%dT%H:%M:%S%z')] $*" | tee -a "$LOG"; }

# 多重起動防止。launchd の StartInterval と前回実行（ハング含む）が重なっても二重 fetch しない。
# 素の macOS に flock は同梱されないため、flock 不在時は mkdir の原子性で排他するフォールバックを
# 必ず効かせる（cron 代替経路でもノーガードにしない）。
# ロックパスは WORKDIR に依存させず固定にする。launchd は WORKDIR=/tmp/paddock-prefetch、手動実行は
# $TMPDIR 配下と WORKDIR が異なるため、WORKDIR 配下に置くと両者が別ロックになり二重 fetch しうる。
# 排他が要るのは実 fetch だけなので、取得は dry-run 早期 return の後（fetch 直前）で行う。
# こうすると read-only な --dry-run は launchd 実走中でもロックに阻まれず常に選択結果を表示できる。
LOCK="/tmp/paddock-prefetch.lock"
LOCK_DIR="/tmp/paddock-prefetch.lock.d"
acquire_lock() {
  if command -v flock >/dev/null 2>&1; then
    exec 9>"$LOCK"
    flock -n 9 || { log "別の prefetch 実行中のためスキップ"; exit 0; }
  else
    # 異常終了でロックが残ると永久ブロックするため、一定時間より古いロックは奪う（前回が
    # ハング/強制終了した残骸とみなす）。閾値は StartInterval(5分) より十分長い 30 分。
    if [ -d "$LOCK_DIR" ] && [ -n "$(find "$LOCK_DIR" -prune -mmin +30 2>/dev/null)" ]; then
      log "古いロックを破棄（前回が異常終了した可能性）: $LOCK_DIR"
      rmdir "$LOCK_DIR" 2>/dev/null || true
    fi
    if ! mkdir "$LOCK_DIR" 2>/dev/null; then
      log "別の prefetch 実行中のためスキップ（mkdir ロック）"
      exit 0
    fi
    trap 'rmdir "$LOCK_DIR" 2>/dev/null || true' EXIT
  fi
}

# paddock race_id（例 2026-3-tokyo-5-6R）→ netkeiba 12 桁。正本は
# src/use-case/src/netkeiba_race_id.rs（CLI 露出が無いため refresh_ev.sh と同じ変換を持つ）。
nk_id() {
  python3 - "$1" <<'PY'
import sys
pid = sys.argv[1]
parts = pid.split("-")  # {年}-{回}-{場slug}-{日}-{R}R
if len(parts) != 5:
    sys.exit(f"nk_id: 想定外の race_id 形式: {pid}")
year, kai, ven, day, rr = parts
vc = {"sapporo": "01", "hakodate": "02", "fukushima": "03", "niigata": "04", "tokyo": "05",
      "nakayama": "06", "chukyo": "07", "kyoto": "08", "hanshin": "09", "kokura": "10"}.get(ven)
if vc is None:
    sys.exit(f"nk_id: 未知の場 slug: {ven}（pid={pid}）")
print(f"{year}{vc}{int(kai):02d}{int(day):02d}{int(rr.rstrip('R')):02d}")
PY
}

# 対象 paddock race_id を DB post_time で選択（#235）。--at はテスト/検証用に現在時刻を上書き。
# command substitution で受けて選択の成否を明示判定する。process substitution（< <(...)）だと
# psql 接続失敗（DB ダウン）でも非0終了が伝播せず「対象0件」と区別不能になり、無人 prefetch が
# 黙って機能停止してもログ上は正常に見えてしまう（Reviewer 指摘）。
SELECT_ARGS=(--window-min "$WINDOW_MIN")
[ -n "$AT" ] && SELECT_ARGS+=(--at "$AT")
if ! SELECTED="$(PADDOCK_DB_URL="$DB_URL" PYTHONPATH="$SCRIPT_DIR" \
      python3 "$SCRIPT_DIR/upcoming_races_db.py" "$DATE" "${SELECT_ARGS[@]}")"; then
  # 失敗要因は DB 接続不可・クエリ失敗のほか、暦上不正な日付（python 側 valid_date が弾く）も
  # ありうるため、原因を断定しない中立な文言にする（「対象0件」とは区別して必ず中断する）。
  log "レース選択コマンドに失敗（DB 接続不可・日付不正・クエリ失敗等）。中断する。"
  exit 1
fi
PIDS=()
while IFS= read -r line; do
  [ -n "$line" ] && PIDS+=("$line")
done <<< "$SELECTED"

if [ "${#PIDS[@]}" -eq 0 ]; then
  log "対象レースなし: $DATE 発走 ${WINDOW_MIN} 分以内の未発走は無し（開催外/朝/全レース終了）"
  exit 0
fi

if [ "$DRY_RUN" -eq 1 ]; then
  log "[dry-run] 対象 ${#PIDS[@]} レース: ${PIDS[*]}"
  exit 0
fi

# ここから実 fetch。多重起動防止のロックを取得（read-only な選択・dry-run は阻まない）。
acquire_lock

# release バイナリ確認（debug ビルドでのライブ運用を防ぐ, refresh_ev.sh と同方針 #211）。
# 実フェッチ時のみ必要なので dry-run の後に置く。
FETCH_BIN="$REPO_ROOT/target/release/paddock-fetch-card"
if [[ ! -x "$FETCH_BIN" ]]; then
  log "release バイナリが見つかりません: $FETCH_BIN"
  log "先に: cd $REPO_ROOT && cargo build --release --bin paddock-fetch-card"
  exit 1
fi

log "prefetch 開始: $DATE 発走 ${WINDOW_MIN} 分以内 ${#PIDS[@]} レース"
FAILED=()
for pid in "${PIDS[@]}"; do
  # race_id 変換失敗（未知 slug 等の異常データ）は 1 件スキップに留め、残りの締切前 prefetch を
  # 巻き添えで止めない（set -e 下の代入失敗で全体中断するのを防ぐ）。
  if ! nk="$(nk_id "$pid")"; then
    log "  SKIP $pid (race_id 変換失敗)"; FAILED+=("$pid"); continue
  fi
  # --force で再取得（既存 race_odds を最新で上書き＋snapshots へ追記）、--skip-history で近走は省く。
  if "$FETCH_BIN" "$nk" --force --skip-history --interval 800 >> "$LOG" 2>&1; then
    log "  ok   $pid ($nk)"
  else
    log "  FAIL $pid ($nk)"; FAILED+=("$pid")
  fi
  sleep 1  # netkeiba への pacing（feedback_jra_fetch_pacing）。fetch-card 内 --interval とは別。
done

if [ "${#FAILED[@]}" -gt 0 ]; then
  log "prefetch 完了（${#FAILED[@]} 件失敗: ${FAILED[*]}）"
else
  log "prefetch 完了（全 ${#PIDS[@]} レース成功）"
fi
