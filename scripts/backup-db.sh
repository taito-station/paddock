#!/usr/bin/env bash
# paddock DB（race_odds_snapshots 等の蓄積資産）を durable な場所へ退避する（#265）。
#
# race_odds_snapshots は Colima の named volume paddock-pgdata 1 か所にしか無く、過去オッズは
# 再取得不能。volume 喪失（Colima reset / docker volume rm / ディスク障害）に備え、full DB を
# custom-format（-Fc・圧縮込み）で dump しタイムスタンプ付きで退避＋世代管理する。
# 復元手順は deployments/db/BACKUP.md。日次実行は deployments/launchd/com.paddock.backup-db.plist。
#
# 二段構成（launchd 対策）:
#   - BACKUP_DIR（ローカル・権威）: dump 本体を置き、世代管理（列挙→剪定）はここで行う。launchd から
#     でもローカル dir は確実に列挙・削除できるため、権威側は常に KEEP 世代に bounded。主脅威である
#     Colima volume 喪失（reset / docker volume rm）はこのローカル退避だけで自動的に外せる。
#   - MIRROR_DIR（iCloud 等・off-machine durable）: 各 dump を cp してディスク障害にも備える。
#   注意: launchd から実行すると **iCloud への "列挙" も "削除" も信頼できない**（書き込み=cp は効くが
#   ls/glob は空を返し rm も反映されない macOS file-provider の癖・検証で確認）。そのため iCloud ミラーの
#   剪定は best-effort（terminal 実行時のみ確実に効き、launchd 下では no-op で溜まる）。iCloud を KEEP に
#   揃えたいときは時々 terminal から本スクリプトを実行して reconcile する。権威（ローカル）は常に bounded。
#
# 重要: host の pg_dump が PG17 サーバより古い（v14 等）とダンプを拒否するため、**dump は
# container 内の pg_dump（バージョン一致）を docker exec で実行**する（host に pg17 client 不要）。
#
# 使い方:
#   scripts/backup-db.sh                 # ローカルへ退避＋iCloud へミラー
#   PADDOCK_BACKUP_DIR=/path scripts/backup-db.sh
#   PADDOCK_BACKUP_MIRROR_DIR="" scripts/backup-db.sh   # ミラー無効（ローカルのみ）
set -euo pipefail

usage() {
    cat <<'EOF'
backup-db.sh - paddock DB を durable な場所へ退避する（#265）

使い方:
  scripts/backup-db.sh            # ローカル権威 dir へ退避＋世代管理し、iCloud へミラー
  scripts/backup-db.sh -h|--help

環境変数:
  PADDOCK_BACKUP_DIR         ローカル権威の退避先（列挙・剪定はここで行う）
                             （既定: ~/paddock-backups）
  PADDOCK_BACKUP_MIRROR_DIR  off-machine ミラー先（ディスク障害対策）。空文字で無効
                             （既定: ~/Library/Mobile Documents/com~apple~CloudDocs/paddock-backups）
  PADDOCK_BACKUP_KEEP        保持する世代数（既定: 14。超過分の古い dump をローカル/ミラー両方から削除）
  PADDOCK_PG_CONTAINER       Postgres コンテナ名（既定: paddock-postgres）
  PADDOCK_PG_USER            DB ユーザ（既定: paddock）
  PADDOCK_PG_DB              DB 名（既定: paddock）
EOF
}

case "${1:-}" in
    -h|--help) usage; exit 0 ;;
    "") ;;
    *) echo "不明な引数: $1" >&2; usage >&2; exit 2 ;;
esac

BACKUP_DIR="${PADDOCK_BACKUP_DIR:-$HOME/paddock-backups}"
MIRROR_DIR="${PADDOCK_BACKUP_MIRROR_DIR-$HOME/Library/Mobile Documents/com~apple~CloudDocs/paddock-backups}"
KEEP="${PADDOCK_BACKUP_KEEP:-14}"
CONTAINER="${PADDOCK_PG_CONTAINER:-paddock-postgres}"
PG_USER="${PADDOCK_PG_USER:-paddock}"
PG_DB="${PADDOCK_PG_DB:-paddock}"

# KEEP は正整数のみ許可（0 や非整数だと世代管理で作成直後の dump ごと全削除する footgun）。
if ! [[ "$KEEP" =~ ^[1-9][0-9]*$ ]]; then
    echo "PADDOCK_BACKUP_KEEP は正整数である必要がある: $KEEP" >&2
    exit 2
fi

command -v docker >/dev/null || { echo "docker が見つからない（PATH を確認）" >&2; exit 1; }
# 起動確認。パイプ+grep -q は pipefail 下で SIGPIPE により誤判定しうるため、一旦変数へ受けてから
# 固定文字列(-F)・完全一致(-x)で照合する。
running_containers="$(docker ps --format '{{.Names}}')"
if ! grep -qxF "$CONTAINER" <<<"$running_containers"; then
    echo "コンテナ $CONTAINER が起動していない（docker compose -f deployments/compose.yaml up -d postgres）" >&2
    exit 1
fi

mkdir -p "$BACKUP_DIR"
ts="$(date +%Y%m%d-%H%M%S)"
base="paddock-$ts.dump"
final="$BACKUP_DIR/$base"
tmp="$final.part"
trap 'rm -f "$tmp"' EXIT

# container 内 pg_dump（バージョン一致）で full DB を custom-format 退避。stdout をホストファイルへ。
# 一時ファイル(.part)に書き、成功＋非空を確認してから最終名へ mv（中断で壊れた dump を残さない）。
if ! docker exec "$CONTAINER" pg_dump -U "$PG_USER" -d "$PG_DB" -Fc --no-owner --no-privileges > "$tmp"; then
    echo "pg_dump に失敗（container=$CONTAINER）" >&2
    exit 1
fi
if [[ ! -s "$tmp" ]]; then
    echo "dump が空（pg_dump は成功したが 0 バイト）" >&2
    exit 1
fi
mv "$tmp" "$final"
trap - EXIT

size="$(du -h "$final" | cut -f1)"
echo "退避完了: $final ($size)"

# off-machine ミラー（best-effort）。cp はパス指定の書き込みなので launchd 下でも効く。失敗しても
# ローカル退避は成功しているので警告に留める（ログで検知する）。
if [[ -n "$MIRROR_DIR" ]]; then
    if mkdir -p "$MIRROR_DIR" && cp -f "$final" "$MIRROR_DIR/$base"; then
        echo "ミラー完了: $MIRROR_DIR/$base"
    else
        echo "警告: ミラーに失敗（ローカル退避は成功。$MIRROR_DIR を確認）" >&2
    fi
fi

# 世代管理: 指定 dir を独立に列挙し新しい順に KEEP 個を残して超過分を削除する。権威(ローカル)と
# ミラー(iCloud)の両方に同じロジックを適用する（ミラーも独立列挙するので、launchd 下で溜まった
# iCloud の蓄積を terminal 実行時に真に KEEP まで reconcile できる）。launchd 下では iCloud 列挙が
# 空を返すためミラー側は自然に no-op（＝溜まる。terminal で回収する）。ローカルは常に KEEP に bounded。
# macOS 既定の /bin/bash 3.2（launchd もこれを使う）に mapfile が無いため while-read で読む。
prune_dir() {
    local dir="$1" label="$2"
    local files=() f i=0
    while IFS= read -r f; do
        [[ -n "$f" ]] && files+=("$f")
    done < <(ls -1t "$dir"/paddock-*.dump 2>/dev/null || true)
    if (( ${#files[@]} > KEEP )); then
        for f in "${files[@]}"; do
            if (( i >= KEEP )); then
                rm -f "$f"
                echo "古い世代を削除($label): $(basename "$f")"
            fi
            i=$((i + 1))
        done
    fi
    local kept=$(( ${#files[@]} > KEEP ? KEEP : ${#files[@]} ))
    echo "保持世代数($label): $kept / KEEP=$KEEP  $dir"
}

prune_dir "$BACKUP_DIR" "権威"
if [[ -n "$MIRROR_DIR" ]]; then
    prune_dir "$MIRROR_DIR" "ミラー"
fi
