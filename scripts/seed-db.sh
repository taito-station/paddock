#!/usr/bin/env bash
# 並走クローン/worktree の data/ に golden DB の一貫スナップショットを配置する(#120)。
#
# 各クローンは DB を共有しておらず（PADDOCK_DB_URL 既定 sqlite://data/paddock.db?mode=rwc は
# 相対パス＝cwd 配下）、並走先は空になる。predict/backtest/analyze を実データで回すたびに
# フル re-ingest する代わりに、ingest 済みの primary clone から即座に seed する。
#
# 既定の golden 元: primary clone（git rev-parse --git-common-dir から自動検出）の data/paddock.db。
# 上書き: --from <path> または環境変数 PADDOCK_GOLDEN_DB。
#
# WAL の取り込み: sqlite3 の .backup（オンラインバックアップ API）を使う。実行中ソースでも
# コミット済み状態の一貫スナップショットを 1 ファイルに作り、target に WAL/SHM 残骸を残さない。
set -euo pipefail

usage() {
    cat <<'EOF'
seed-db.sh - 並走クローンの data/ に golden DB のスナップショットを配置する(#120)

使い方:
  scripts/seed-db.sh                       # primary を自動検出して ./data に seed
  scripts/seed-db.sh --from /path/to.db    # golden を明示
  scripts/seed-db.sh --to /other/data      # 配置先 data ディレクトリを明示
  PADDOCK_GOLDEN_DB=/path/to.db scripts/seed-db.sh

オプション:
  --from <path>   golden DB（省略時: PADDOCK_GOLDEN_DB → primary clone 自動検出）
  --to <dir>      配置先 data ディレクトリ（既定: data）
  -h, --help      このヘルプを表示
EOF
}

FROM=""
TO="data"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --from) FROM="${2:?--from にパスが必要}"; shift 2 ;;
        --to)   TO="${2:?--to にパスが必要}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "不明な引数: $1" >&2; usage >&2; exit 2 ;;
    esac
done

# golden 元の解決: --from > PADDOCK_GOLDEN_DB > primary clone 自動検出。
if [[ -z "$FROM" ]]; then
    FROM="${PADDOCK_GOLDEN_DB:-}"
fi
if [[ -z "$FROM" ]]; then
    common_dir="$(git rev-parse --git-common-dir 2>/dev/null || true)"
    if [[ -n "$common_dir" ]]; then
        # common_dir は cwd 相対のことがある（primary の root/サブディレクトリ）。
        # cd && pwd で絶対パスへ正規化すると root/サブいずれから実行しても primary を指す。
        FROM="$(cd "$(dirname "$common_dir")" && pwd)/data/paddock.db"
    fi
fi

if [[ -z "$FROM" || ! -f "$FROM" ]]; then
    echo "golden DB が見つからない: '${FROM:-<未指定>}'" >&2
    echo "--from <path> か PADDOCK_GOLDEN_DB で ingest 済みの paddock.db を指定する" >&2
    exit 1
fi

command -v sqlite3 >/dev/null || { echo "sqlite3 が見つからない" >&2; exit 1; }

mkdir -p "$TO"
DEST="$TO/paddock.db"

# 自己 seed（golden と target が同一実体）を防ぐ。
src_abs="$(cd "$(dirname "$FROM")" && pwd)/$(basename "$FROM")"
dest_abs="$(cd "$TO" && pwd)/paddock.db"
if [[ "$src_abs" == "$dest_abs" ]]; then
    echo "golden と配置先が同一: $dest_abs。別クローンから seed する" >&2
    exit 1
fi

# まず一時ファイルへ一貫スナップショットを作り、検証が通ってから本配置する。
# こうすると .backup や検証が失敗しても既存 DB を壊さない（非破壊）。
tmp="$DEST.seed-tmp.$$"
rm -f "$tmp" "$tmp-wal" "$tmp-shm"
trap 'rm -f "$tmp" "$tmp-wal" "$tmp-shm"' EXIT

# .backup のパスは sqlite のドット引数（SQL 文字列リテラル）として解釈されるため、
# シェルクォートではなく SQL のクォート（' を '' に二重化）でエスケープする。
tmp_sql="${tmp//\'/\'\'}"
sqlite3 "$FROM" ".backup '$tmp_sql'"

# 本配置前にスナップショットの中身を検証する（破損/スキーマ不一致を握りつぶさない）。
races="$(sqlite3 "$tmp" 'SELECT COUNT(*) FROM races;' 2>/dev/null || true)"
if [[ -z "$races" ]]; then
    echo "スナップショットから races を読めなかった（golden が破損 / スキーマ不一致の可能性）: $FROM" >&2
    exit 1
fi

# 検証クエリが作った空の WAL/SHM を畳んで単一ファイルにする（app 起動時に再作成される）。
sqlite3 "$tmp" 'PRAGMA wal_checkpoint(TRUNCATE);' >/dev/null 2>&1 || true
rm -f "$tmp-wal" "$tmp-shm"

# ここで初めて既存 DB と WAL/SHM 残骸を .bak へ退避し、一時ファイルを本配置する。
ts="$(date +%Y%m%d-%H%M%S)"
for f in "$DEST" "$DEST-wal" "$DEST-shm"; do
    if [[ -e "$f" ]]; then
        mv "$f" "$f.bak-$ts"
        echo "退避: $f -> $f.bak-$ts"
    fi
done
mv "$tmp" "$DEST"
trap - EXIT

size="$(du -h "$DEST" | cut -f1)"
echo "seeded: $FROM -> $DEST ($size, races=$races)"
