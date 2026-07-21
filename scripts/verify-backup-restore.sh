#!/usr/bin/env bash
# paddock DB バックアップ restore 検証スクリプト（#474）。
#
# 最新の dump を scratch DB（コンテナ内・使い捨て）へ復元し、主要テーブルの行数を
# golden DB と突合することで「復元できない dump を守っていた」状態を週次で検知する。
#
# race_odds_snapshots は再取得不能資産であり、restore 可能性こそがバックアップの本体。
#
# 検証フロー:
#   1. BACKUP_DIR から最新 dump を特定
#   2. コンテナ内に scratch DB（paddock_restore_verify_YYYYMMDD）を作成
#   3. dump を pg_restore で scratch DB へ復元
#   4. 各突合テーブルの行数を scratch / golden 両方から取得して比較
#   5. scratch DB を必ず削除（golden は read-only で一切触れない）
#   6. 不一致・エラーを通知＋ログ出力
#
# 注意: golden DB（paddock）には一切書き込まない。scratch は このスクリプトが CREATE/DROP する。
#
# 使い方:
#   scripts/verify-backup-restore.sh               # BACKUP_DIR の最新 dump を検証
#   PADDOCK_VERIFY_DUMP=/path/to/xxx.dump \
#     scripts/verify-backup-restore.sh             # 特定 dump を指定して検証
#   scripts/verify-backup-restore.sh -h|--help
#
# 環境変数:
#   PADDOCK_BACKUP_DIR      dump が置かれている dir（既定: ~/paddock-backups）
#   PADDOCK_VERIFY_DUMP     使用する dump ファイルを直接指定（指定時は BACKUP_DIR を無視）
#   PADDOCK_PG_CONTAINER    Postgres コンテナ名（既定: paddock-postgres）
#   PADDOCK_PG_USER         DB ユーザ（既定: paddock）
#   PADDOCK_PG_DB           golden DB 名（既定: paddock）
#   PADDOCK_VERIFY_TABLES   突合するテーブル（カンマ区切り。既定: 下記 DEFAULT_TABLES）
#
# 週次スケジュール: deployments/launchd/com.paddock.verify-backup-restore.plist（日曜 04:00）
set -euo pipefail

log() { echo "[$(date '+%Y-%m-%dT%H:%M:%S%z')] $*"; }

notify() {
    # メッセージは argv 経由で AppleScript に渡す（文字列補間だと " / \ で壊れるため）。
    osascript -e 'on run {msg}' -e 'display notification msg with title "paddock verify-backup-restore"' -e 'end run' -- "$1" >/dev/null 2>&1 || true
}

# scratch DB 名（作成・削除を完全にこのスクリプトが管理）。
SCRATCH_DB=""
# EXIT ハンドラ: scratch DB が作られていれば必ず削除する（golden を汚さない）。
_cleanup() {
    local rc=$?
    if [[ -n "$SCRATCH_DB" ]]; then
        docker exec "$CONTAINER" dropdb -U "$PG_USER" --if-exists "$SCRATCH_DB" 2>/dev/null || true
        log "scratch DB を削除しました: $SCRATCH_DB"
    fi
    if [[ $rc -ne 0 ]]; then
        log "FAIL: verify-backup-restore exited rc=$rc"
        notify "restore 検証 FAILED (rc=$rc)"
    fi
}
trap '_cleanup' EXIT

usage() {
    cat <<'EOF'
verify-backup-restore.sh - paddock dump の restore 検証（scratch DB で行数突合・#474）

使い方:
  scripts/verify-backup-restore.sh
  scripts/verify-backup-restore.sh -h|--help

環境変数:
  PADDOCK_BACKUP_DIR      dump が置かれている dir（既定: ~/paddock-backups）
  PADDOCK_VERIFY_DUMP     使用する dump ファイルを直接指定（指定時は BACKUP_DIR を無視）
  PADDOCK_PG_CONTAINER    Postgres コンテナ名（既定: paddock-postgres）
  PADDOCK_PG_USER         DB ユーザ（既定: paddock）
  PADDOCK_PG_DB           golden DB 名（既定: paddock）
  PADDOCK_VERIFY_TABLES   突合するテーブル（カンマ区切り。既定: race_odds_snapshots,races,horses）
EOF
}

case "${1:-}" in
    -h|--help) usage; exit 0 ;;
    "") ;;
    *) echo "不明な引数: $1" >&2; usage >&2; exit 2 ;;
esac

BACKUP_DIR="${PADDOCK_BACKUP_DIR:-$HOME/paddock-backups}"
VERIFY_DUMP="${PADDOCK_VERIFY_DUMP:-}"
CONTAINER="${PADDOCK_PG_CONTAINER:-paddock-postgres}"
PG_USER="${PADDOCK_PG_USER:-paddock}"
PG_DB="${PADDOCK_PG_DB:-paddock}"
# 突合テーブル: race_odds_snapshots（再取得不能資産）を先頭に、主要テーブルを検証する。
DEFAULT_TABLES="race_odds_snapshots,races,horses"
VERIFY_TABLES="${PADDOCK_VERIFY_TABLES:-$DEFAULT_TABLES}"

# --- docker 疎通確認 ---
command -v docker >/dev/null || { echo "docker が見つからない（PATH を確認）" >&2; exit 1; }
running_containers="$(docker ps --format '{{.Names}}')"
if ! grep -qxF "$CONTAINER" <<<"$running_containers"; then
    echo "コンテナ $CONTAINER が起動していない（docker compose -f deployments/compose.yaml up -d postgres）" >&2
    exit 1
fi

# --- dump ファイル特定 ---
if [[ -n "$VERIFY_DUMP" ]]; then
    DUMP_FILE="$VERIFY_DUMP"
    if [[ ! -f "$DUMP_FILE" ]]; then
        echo "指定された dump ファイルが見つからない: $DUMP_FILE" >&2
        exit 1
    fi
else
    # BACKUP_DIR から最新 dump を選ぶ（mtime 降順で先頭）。
    if [[ ! -d "$BACKUP_DIR" ]]; then
        echo "BACKUP_DIR が存在しない: $BACKUP_DIR" >&2
        exit 1
    fi
    DUMP_FILE=""
    while IFS= read -r f; do
        [[ -n "$f" ]] && { DUMP_FILE="$f"; break; }
    done < <(
        find "$BACKUP_DIR" -maxdepth 1 -type f -name 'paddock-*.dump' \
            -exec stat -f '%m %N' {} + 2>/dev/null \
            | sort -t' ' -k1,1rn -k2r | cut -d' ' -f2-
    )
    if [[ -z "$DUMP_FILE" ]]; then
        echo "BACKUP_DIR に dump ファイルが見つからない: $BACKUP_DIR" >&2
        exit 1
    fi
fi

dump_size="$(du -h "$DUMP_FILE" | cut -f1)"
log "使用する dump: $DUMP_FILE ($dump_size)"

# --- scratch DB 作成 ---
ts="$(date +%Y%m%d_%H%M%S)"
SCRATCH_DB="paddock_restore_verify_${ts}"
log "scratch DB を作成: $SCRATCH_DB"
docker exec "$CONTAINER" createdb -U "$PG_USER" "$SCRATCH_DB"

# --- restore ---
log "pg_restore 開始 → $SCRATCH_DB"
if ! docker exec -i "$CONTAINER" pg_restore -U "$PG_USER" -d "$SCRATCH_DB" < "$DUMP_FILE"; then
    # pg_restore は警告を rc=1 で返すことがある（権限系の警告等）。ここでは致命的失敗として扱う。
    echo "pg_restore が異常終了（壊れた dump か restore 失敗）: rc=$?" >&2
    exit 1
fi
log "pg_restore 完了"

# --- 行数突合 ---
failed_tables=()
# PADDOCK_VERIFY_TABLES のカンマ区切りを処理（zsh/bash 両対応: while read -d, で分割）。
while IFS= read -r table; do
    [[ -z "$table" ]] && continue
    # 末尾の改行やスペースを除去。
    table="${table#"${table%%[![:space:]]*}"}"
    table="${table%"${table##*[![:space:]]}"}"
    [[ -z "$table" ]] && continue

    # golden の行数を取得（golden は読み取り専用）。
    golden_count="$(docker exec "$CONTAINER" psql -U "$PG_USER" -d "$PG_DB" -t -A \
        -c "SELECT COUNT(*) FROM $table;" 2>/dev/null || echo "ERROR")"
    scratch_count="$(docker exec "$CONTAINER" psql -U "$PG_USER" -d "$SCRATCH_DB" -t -A \
        -c "SELECT COUNT(*) FROM $table;" 2>/dev/null || echo "ERROR")"

    if [[ "$golden_count" == "ERROR" || "$scratch_count" == "ERROR" ]]; then
        log "WARN: テーブル $table の行数取得に失敗（テーブルが存在しないか権限エラー）"
        log "      golden=$golden_count scratch=$scratch_count"
        failed_tables+=("$table(query_error)")
        continue
    fi

    if [[ "$golden_count" -eq "$scratch_count" ]]; then
        log "OK  $table: golden=$golden_count / scratch=$scratch_count"
    else
        log "MISMATCH $table: golden=$golden_count / scratch=$scratch_count"
        failed_tables+=("$table(golden=${golden_count},scratch=${scratch_count})")
    fi
done < <(printf '%s\n' "${VERIFY_TABLES//,/$'\n'}")

# --- 結果判定 ---
if [[ ${#failed_tables[@]} -gt 0 ]]; then
    log "FAIL: 行数不一致 or クエリエラー: ${failed_tables[*]}"
    notify "restore 検証失敗: 行数不一致 ${failed_tables[*]}"
    exit 1
fi

log "SUCCESS: restore 検証完了。全テーブル行数一致（dump: $(basename "$DUMP_FILE")）"
notify "restore 検証 OK（$(basename "$DUMP_FILE")）"
