#!/usr/bin/env bash
# paddock DB バックアップ restore 検証スクリプト（#474）。
#
# 最新の dump を scratch DB（コンテナ内・使い捨て）へ復元し、主要テーブルの行数を
# 「dump 生成時に記録したサイドカー(<dump>.rowcounts)の値」と突合することで、
# 「復元できない dump を守っていた」状態を週次で検知する。
#
# race_odds_snapshots は再取得不能資産であり、restore 可能性こそがバックアップの本体。
#
# なぜ live golden ではなくサイドカーと突合するか（#474 レビュー S1）:
#   検証は dump 生成から数時間後（例: dump=土23:30 / 検証=日04:00）に走る。その間 live golden へ
#   INSERT が入ると「scratch(dump時点) < golden(検証時点)」となり、厳密 -eq 突合だと偽 FAIL する。
#   偽 FAIL は真の警告（本当に壊れた dump）を鈍らせるので避けたい。かといって「scratch>golden のみ
#   FAIL」にすると restore がサイレントに行を落とすケース（scratch<golden）を見逃し、#474 が本来
#   拾いたい「復元で行が欠ける破損」を検知できず本末転倒になる。
#   → dump 生成とほぼ同時刻の行数を backup-db.sh がサイドカーに記録し、検証側はその記録値と
#     厳密 -eq 突合する（race-free）。これなら時刻ズレが原理的に無く、増加も欠落も正しく判定できる。
#   サイドカーが無い旧 dump は skip（警告）にフォールバックする（強制 FAIL にはしない）。
#
# 検証フロー:
#   1. BACKUP_DIR から最新 dump を特定
#   2. 対になるサイドカー(<dump>.rowcounts)を読む（無ければ skip して正常終了）
#   3. コンテナ内に scratch DB（paddock_restore_verify_YYYYMMDD）を作成
#   4. dump を pg_restore で scratch DB へ復元
#   5. 各突合テーブルの scratch 行数を、サイドカー記録値と厳密 -eq 突合
#   6. scratch DB を必ず削除（golden は read-only で一切触れない）
#   7. 不一致・エラーを通知＋ログ出力
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
#   PADDOCK_PG_DB           golden DB 名（既定: paddock。突合は live golden ではなくサイドカーと行う
#                           ため参照は疎通確認・任意用途のみ。行数突合には使わない）
# 突合対象テーブルは dump 生成時に backup-db.sh が記録したサイドカー(<dump>.rowcounts)に従う
# （backup-db.sh 側の PADDOCK_VERIFY_TABLES で決まる。既定 race_odds_snapshots,races,horses）。
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

突合対象テーブルは dump 生成時のサイドカー(<dump>.rowcounts)に従う（backup-db.sh 側の
PADDOCK_VERIFY_TABLES で決まる。既定 race_odds_snapshots,races,horses）。
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
# 突合対象テーブルと期待行数は dump 生成時のサイドカー(<dump>.rowcounts)から読む（後述）。
# live golden とは突合しない（時刻ズレによる偽 FAIL を避けるため・#474 レビュー S1）。

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

# --- サイドカー（dump 生成時点の行数）を読む ---
# 突合の基準は live golden ではなく、この dump と対になる <dump>.rowcounts（backup-db.sh が記録）。
# 書式は "table<TAB>count" の 1 行/テーブル。無ければ「サイドカー無し」として skip（正常終了）。
# 旧 dump（サイドカー機能導入前）は必ずここに来るため、強制 FAIL にせず skip 警告に留める。
ROWCOUNTS_FILE="$DUMP_FILE.rowcounts"
if [[ ! -f "$ROWCOUNTS_FILE" ]]; then
    log "SKIP: 行数サイドカーが無い（旧 dump か記録失敗）: $ROWCOUNTS_FILE"
    log "      restore の構造的健全性は backup-db.sh の pg_restore --list で担保済み。行数突合はスキップ。"
    notify "restore 検証: サイドカー無しでスキップ（$(basename "$DUMP_FILE")）"
    exit 0
fi
# サイドカーの有効行（"table<TAB>count"）を通常配列へ読み込む。空・不正行はスキップ。
# 連想配列(declare -A)は使わない: この plist は /bin/bash（macOS 既定 3.2）で起動され、3.2 は
# 連想配列非対応のため（backup-db.sh が mapfile を避けているのと同じ制約）。要素は "table<TAB>count"
# のまま保持し、突合ループで分割する。
expected_rows=()
while IFS=$'\t' read -r etable ecount; do
    [[ -z "$etable" ]] && continue
    [[ "$ecount" =~ ^[0-9]+$ ]] || continue
    expected_rows+=("$etable"$'\t'"$ecount")
done < "$ROWCOUNTS_FILE"
if [[ ${#expected_rows[@]} -eq 0 ]]; then
    log "SKIP: 行数サイドカーが空/不正（有効な行が無い）: $ROWCOUNTS_FILE"
    notify "restore 検証: サイドカー空でスキップ（$(basename "$DUMP_FILE")）"
    exit 0
fi
log "サイドカー読込: ${#expected_rows[@]} テーブル（$ROWCOUNTS_FILE）"

# --- scratch DB 作成 ---
ts="$(date +%Y%m%d_%H%M%S)"
SCRATCH_DB="paddock_restore_verify_${ts}"
log "scratch DB を作成: $SCRATCH_DB"
docker exec "$CONTAINER" createdb -U "$PG_USER" "$SCRATCH_DB"

# --- restore ---
# set -e 下で実 exit code を捕捉するため `|| restore_rc=$?` で握る（`if ! ...` だと $? が常に 0 で
# ログが無意味になる・S2）。pg_restore は権限系の警告でも rc=1 を返すことがあるが、ここでは
# 致命的失敗として扱う（scratch は空 DB で権限警告は出ない前提）。
log "pg_restore 開始 → $SCRATCH_DB"
restore_rc=0
docker exec -i "$CONTAINER" pg_restore -U "$PG_USER" -d "$SCRATCH_DB" < "$DUMP_FILE" || restore_rc=$?
if [[ $restore_rc -ne 0 ]]; then
    echo "pg_restore が異常終了（壊れた dump か restore 失敗）: rc=$restore_rc" >&2
    exit 1
fi
log "pg_restore 完了"

# --- 行数突合（scratch 復元行数 vs サイドカー記録値・厳密 -eq）---
failed_tables=()
for row in "${expected_rows[@]}"; do
    table="${row%%$'\t'*}"    # TAB より前 = テーブル名
    expected="${row##*$'\t'}"  # TAB より後 = 期待行数
    # scratch の実行数を COUNT(*) で取得（golden には一切触れない）。
    scratch_rc=0
    scratch_count="$(docker exec "$CONTAINER" psql -U "$PG_USER" -d "$SCRATCH_DB" -t -A \
        -c "SELECT COUNT(*) FROM $table;" 2>/dev/null)" || scratch_rc=$?
    if [[ $scratch_rc -ne 0 || -z "$scratch_count" || ! "$scratch_count" =~ ^[0-9]+$ ]]; then
        log "WARN: テーブル $table の scratch 行数取得に失敗（テーブル欠落か復元不備）: '$scratch_count'"
        failed_tables+=("$table(query_error)")
        continue
    fi

    # dump 生成時点の記録値と厳密比較。時刻ズレが無いので増加・欠落いずれも正しく検知できる。
    if [[ "$scratch_count" -eq "$expected" ]]; then
        log "OK  $table: expected(dump)=$expected / scratch=$scratch_count"
    else
        log "MISMATCH $table: expected(dump)=$expected / scratch=$scratch_count"
        failed_tables+=("$table(expected=${expected},scratch=${scratch_count})")
    fi
done

# --- 結果判定 ---
if [[ ${#failed_tables[@]} -gt 0 ]]; then
    log "FAIL: 行数不一致 or クエリエラー: ${failed_tables[*]}"
    notify "restore 検証失敗: 行数不一致 ${failed_tables[*]}"
    exit 1
fi

log "SUCCESS: restore 検証完了。全 ${#expected_rows[@]} テーブルの行数がサイドカー記録値と一致（dump: $(basename "$DUMP_FILE")）"
notify "restore 検証 OK（$(basename "$DUMP_FILE")）"
