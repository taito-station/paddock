#!/usr/bin/env bash
# 対象 worktree の DB を空に戻す（再 seed / 再 ingest 前提）(#36 Postgres 版)。
#
# 対象 database を DROP/CREATE して空にする。次回のアプリ起動（pool::migrate）でスキーマが
# 再生成される。seed-db.sh と対称に、golden（既定: 同サーバの paddock DB）への reset は
# 既定で中断する（--force で明示的に許可）。
#
# 前提: psql（libpq クライアント）が要る。対象 DB を使用中のアプリは停止しておく。
set -euo pipefail

TO_URL="${PADDOCK_DB_URL:-}"
GOLDEN_URL="${PADDOCK_GOLDEN_DB_URL:-postgres://paddock:paddock@localhost:5432/paddock}"
FORCE=0

usage() {
    cat <<'EOF'
reset-db.sh - 対象 worktree の DB を空に戻す（Postgres）

対象 database を DROP/CREATE して空にする。次回のアプリ起動で自動マイグレートされる。

使い方:
  scripts/reset-db.sh                 # $PADDOCK_DB_URL の database を空に戻す
  scripts/reset-db.sh --to <url>      # 対象を明示
  scripts/reset-db.sh --force         # golden(paddock) への reset も許可する

オプション:
  --to <url>   対象 DB の接続 URL（既定: PADDOCK_DB_URL）
  --force      golden DB への reset を許可する
  -h, --help   このヘルプ
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --to) TO_URL="${2:?--to に URL が必要}"; shift 2 ;;
        --force) FORCE=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "不明な引数: $1" >&2; usage >&2; exit 2 ;;
    esac
done

if [[ -z "$TO_URL" ]]; then
    echo "対象が未指定: PADDOCK_DB_URL を .env で設定するか --to <url> を渡す" >&2
    exit 1
fi
command -v psql >/dev/null || { echo "psql が見つからない" >&2; exit 1; }

# golden への誤爆を防ぐ（クエリ文字列を無視して比較）。
if [[ "$FORCE" -ne 1 && "${TO_URL%%\?*}" == "${GOLDEN_URL%%\?*}" ]]; then
    echo "対象が golden DB です: $TO_URL。全 worktree の seed 元を失うため既定では中断する。" >&2
    echo "意図的なら --force を付ける" >&2
    exit 1
fi

# 対象 URL から database 名と管理用 URL（同サーバの postgres DB）を導出する。
to_noq="${TO_URL%%\?*}"
target_db="${to_noq##*/}"
admin_url="${to_noq%/*}/postgres"
if [[ -z "$target_db" || "$target_db" == "$to_noq" ]]; then
    echo "対象 URL から database 名を取得できない: $TO_URL" >&2
    exit 1
fi

psql "$admin_url" -v ON_ERROR_STOP=1 -q \
    -c "DROP DATABASE IF EXISTS \"$target_db\" WITH (FORCE);" \
    -c "CREATE DATABASE \"$target_db\";"

echo "reset 完了: $target_db を空に戻した（次回アプリ起動で自動マイグレート / seed-db.sh で複製）"
