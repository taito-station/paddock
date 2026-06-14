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
#
# 使い方:
#   scripts/seed-db.sh                       # primary を自動検出して ./data に seed
#   scripts/seed-db.sh --from /path/to.db    # golden を明示
#   scripts/seed-db.sh --to /other/data      # 配置先 data ディレクトリを明示
#   PADDOCK_GOLDEN_DB=/path/to.db scripts/seed-db.sh
set -euo pipefail

FROM=""
TO="data"

usage() {
    sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'
}

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

# 配置前に既存 DB と WAL/SHM 残骸を .bak へ退避（誤って消さないため）。
ts="$(date +%Y%m%d-%H%M%S)"
for f in "$DEST" "$DEST-wal" "$DEST-shm"; do
    if [[ -e "$f" ]]; then
        mv "$f" "$f.bak-$ts"
        echo "退避: $f -> $f.bak-$ts"
    fi
done

# 一貫スナップショットを配置。.backup はコミット済み状態を 1 ファイルに畳み込むため、
# 旧版のように不整合な WAL/SHM をコピーしてしまうことはない。
sqlite3 "$FROM" ".backup '$DEST'"

size="$(du -h "$DEST" | cut -f1)"
races="$(sqlite3 "$DEST" 'SELECT COUNT(*) FROM races;' 2>/dev/null || echo '?')"

# 上の参照クエリが作った空の WAL/SHM を片付け、配置結果を単一ファイルに保つ
# （app 起動時に WAL モードで開き直されると再作成される）。
sqlite3 "$DEST" 'PRAGMA wal_checkpoint(TRUNCATE);' >/dev/null 2>&1 || true
rm -f "$DEST-wal" "$DEST-shm"

echo "seeded: $FROM -> $DEST ($size, races=$races)"
