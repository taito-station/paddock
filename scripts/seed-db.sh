#!/usr/bin/env bash
# 並走 worktree の DB に golden DB の内容を複製する（#36 Postgres 版）。
#
# 各 worktree は 1 つの PG サーバ（deployments/compose.yaml）を共有し、worktree ごとに別
# database 名で分離する（PADDOCK_DB_URL の DB 名を変える）。ingest 済みの golden DB（既定:
# 同サーバの paddock DB）を pg_dump して配置先 DB を丸ごと作り直す。pg_dump は _sqlx_migrations
# も含むため、配置後のアプリ起動で再マイグレーションは走らない（チェックサム一致）。
#
# 前提: psql / pg_dump（libpq クライアント）が要る。pg_dump の**メジャー版はサーバ（PG 17）以上**
# が必要（古い pg_dump は新しいサーバをダンプ拒否する）。例: `brew install postgresql@17` で 17 系を入れ
# `$(brew --prefix postgresql@17)/bin` を PATH 前方に置く。配置先 DB を使用中のアプリは停止しておく
# （DROP DATABASE ... WITH (FORCE) で接続は切断するが、稼働中プロセスは再接続しうる）。
set -euo pipefail

usage() {
    cat <<'EOF'
seed-db.sh - 並走 worktree の DB に golden DB を複製する（Postgres）

使い方:
  scripts/seed-db.sh                          # golden(paddock) → $PADDOCK_DB_URL へ複製
  scripts/seed-db.sh --from <golden_url>      # golden を明示
  scripts/seed-db.sh --to <target_url>        # 配置先を明示（既定: $PADDOCK_DB_URL）
  PADDOCK_GOLDEN_DB_URL=<url> scripts/seed-db.sh

オプション:
  --from <url>  golden DB の接続 URL（既定: PADDOCK_GOLDEN_DB_URL → postgres://paddock:paddock@localhost:5432/paddock）
  --to <url>    配置先 DB の接続 URL（既定: PADDOCK_DB_URL）
  -h, --help    このヘルプ

前提: golden と配置先は同一 PG サーバ上にある想定（配置先の DROP/CREATE には配置先サーバの
postgres DB へ管理接続する）。別サーバの golden から複製する用途は非対応。
EOF
}

FROM_URL="${PADDOCK_GOLDEN_DB_URL:-}"
TO_URL="${PADDOCK_DB_URL:-}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --from) FROM_URL="${2:?--from に URL が必要}"; shift 2 ;;
        --to)   TO_URL="${2:?--to に URL が必要}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "不明な引数: $1" >&2; usage >&2; exit 2 ;;
    esac
done

[[ -n "$FROM_URL" ]] || FROM_URL="postgres://paddock:paddock@localhost:5432/paddock"
if [[ -z "$TO_URL" ]]; then
    echo "配置先が未指定: PADDOCK_DB_URL を .env で設定するか --to <url> を渡す" >&2
    exit 1
fi
# クエリ文字列（?sslmode=... 等）を剥がして比較する（reset-db.sh の golden ガードと対称）。
# 同一 DB をクエリだけ違う URL で指したときに golden を上書きしないため。
if [[ "${FROM_URL%%\?*}" == "${TO_URL%%\?*}" ]]; then
    echo "golden と配置先が同一 DB: $TO_URL。別 database を配置先にする" >&2
    exit 1
fi

command -v psql >/dev/null    || { echo "psql が見つからない" >&2; exit 1; }
command -v pg_dump >/dev/null || { echo "pg_dump が見つからない" >&2; exit 1; }

# golden の sanity check（races が入っているか）。
races="$(psql "$FROM_URL" -tAc 'SELECT COUNT(*) FROM races;' 2>/dev/null || true)"
if ! [[ "$races" =~ ^[0-9]+$ ]] || [[ "$races" -eq 0 ]]; then
    echo "golden に races が無い（空 / 未マイグレート / 接続不可の可能性）: $FROM_URL" >&2
    exit 1
fi

# 配置先 URL から database 名と管理用 URL（同サーバの postgres DB）を導出する。
to_noq="${TO_URL%%\?*}"          # クエリ文字列を除去
target_db="${to_noq##*/}"        # 末尾セグメント = database 名
admin_url="${to_noq%/*}/postgres"
if [[ -z "$target_db" || "$target_db" == "$to_noq" ]]; then
    echo "配置先 URL から database 名を取得できない: $TO_URL" >&2
    exit 1
fi

# golden を一時ファイルへダンプし、成否を確かめてから流し込む（パイプ直結だと pg_dump の
# 途中失敗を取りこぼし、中途半端な DB を「seed 成功」と誤認しうる）。
# X はテンプレート末尾に置く（GNU/BSD 双方で確実に展開させるため）。
dump="$(mktemp "${TMPDIR:-/tmp}/paddock-seed.XXXXXX")"
trap 'rm -f "$dump"' EXIT
# --no-owner / --no-privileges で owner・権限文を落とし、ロール差のある環境でも流し込めるようにする。
if ! pg_dump --no-owner --no-privileges "$FROM_URL" >"$dump"; then
    echo "pg_dump に失敗: $FROM_URL（pg_dump のメジャー版がサーバ未満の可能性）" >&2
    exit 1
fi

# 配置先を作り直す（接続は FORCE で切断。PG13+ 必須）。ダンプ成功後に実施し、失敗時に
# 既存の配置先 DB を壊さない。
psql "$admin_url" -v ON_ERROR_STOP=1 -q \
    -c "DROP DATABASE IF EXISTS \"$target_db\" WITH (FORCE);" \
    -c "CREATE DATABASE \"$target_db\";"

# この段階で配置先は DROP/CREATE 済み（空）。流し込みが失敗するとその空のまま残るため、
# 状態が分かるよう明示する（再実行で作り直される）。
if ! psql "$TO_URL" -v ON_ERROR_STOP=1 -q -f "$dump"; then
    echo "復元に失敗: 配置先 $target_db は空のまま残った。修正のうえ再実行する" >&2
    exit 1
fi

# seed 後の sanity check。pg_dump 全体復元なので部分欠落は通常起きないが、代表 2 表（races/results）
# の件数一致だけ軽量に確認する（全数照合は runbook 手順 4 を参照）。
for t in races results; do
    g="$(psql "$FROM_URL" -tAc "SELECT COUNT(*) FROM $t;" 2>/dev/null || true)"
    d="$(psql "$TO_URL"   -tAc "SELECT COUNT(*) FROM $t;" 2>/dev/null || true)"
    if [[ "$d" != "$g" ]]; then
        echo "seed 後の $t 件数が一致しない（golden=$g, 配置先=${d:-?}）" >&2
        echo "  （稼働中アプリの書き込み / golden の途中更新 / pg_dump 不整合の可能性）" >&2
        exit 1
    fi
done

echo "seeded: $FROM_URL -> $TO_URL (races=$races)"
