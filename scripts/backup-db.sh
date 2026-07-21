#!/usr/bin/env bash
# paddock DB（race_odds_snapshots 等の蓄積資産）を durable な場所へ退避する（#265）。
#
# race_odds_snapshots は Colima の named volume paddock-pgdata 1 か所にしか無く、過去オッズは
# 再取得不能。volume 喪失（Colima reset / docker volume rm / ディスク障害）に備え、full DB を
# custom-format（-Fc・圧縮込み）で dump しタイムスタンプ付きで退避＋世代管理する。
# 復元手順は deployments/db/BACKUP.md。日次実行は deployments/launchd/com.paddock.backup-db.plist。
#
# 退避先: BACKUP_DIR（ローカル・権威）に dump 本体を置き、世代管理（列挙→剪定）を行う。launchd から
# でもローカル dir は確実に列挙・削除できるため、権威側は常に KEEP 世代に bounded。主脅威である
# Colima volume 喪失（reset / docker volume rm）はこのローカル退避だけで自動的に外せる。
#
# off-machine ミラー（既定 off・オプトイン）: PADDOCK_BACKUP_MIRROR_DIR を指定すると各 dump をそこへ
# cp してディスク障害にも備える。ミラー先には **実ファイルシステム（外付け/NAS 等）** を使うこと。
# iCloud Drive は使わない: launchd から実行すると iCloud への "列挙" も "削除" も信頼できず（書き込み=
# cp は効くが ls/glob は空を返し rm も反映されない macOS file-provider の癖・検証で確認）、剪定が no-op
# になって無制限に溜まるため（#494）。既定はミラー無効でローカル権威のみ。
#
# 重要: host の pg_dump が PG17 サーバより古い（v14 等）とダンプを拒否するため、**dump は
# container 内の pg_dump（バージョン一致）を docker exec で実行**する（host に pg17 client 不要）。
#
# 使い方:
#   scripts/backup-db.sh                                        # ローカル権威のみへ退避（既定）
#   PADDOCK_BACKUP_DIR=/path scripts/backup-db.sh
#   PADDOCK_BACKUP_MIRROR_DIR=/Volumes/ext/paddock-backups scripts/backup-db.sh  # off-machine ミラー有効
set -euo pipefail

log() { echo "[$(date '+%Y-%m-%dT%H:%M:%S%z')] $*"; }

notify() {
    # メッセージは argv 経由で AppleScript に渡す（文字列補間だと、パスやファイル名に含まれる
    # " / \ で AppleScript 文字列が壊れて通知が化けるため）。表示不可環境でも本処理は止めない。
    osascript -e 'on run {msg}' -e 'display notification msg with title "paddock backup"' -e 'end run' -- "$1" >/dev/null 2>&1 || true
}

# 一時ファイルパス（未確定の段階は空文字。EXIT ハンドラで rm -f しても無害）。
_tmp=""
# 終了コード（shellcheck SC2154 対策でスクリプト冒頭で初期化。実値は EXIT ハンドラ内で $? から取る）。
_rc=0
# 入力検証（引数・KEEP 値）を通過したら 1 にする。使い方/入力エラーの exit 2 は必ず検証完了前
# に起きる（_validated=0）ので、それだけを FAIL 通知から除外する。検証通過後（_validated=1）に
# 何らかの理由で rc=2 が出た場合は実失敗として通知する（偶発 rc=2 の握りつぶし防止）。
_validated=0
# rc=1（docker/pg_dump/空 dump 等の実失敗）は常に通知。rc=2 は _validated=0 のとき（使い方
# エラー）のみ通知対象外。
trap '_rc=$?; [ -n "$_tmp" ] && rm -f "$_tmp"; if [ "$_rc" -ne 0 ] && { [ "$_rc" -ne 2 ] || [ "$_validated" -eq 1 ]; }; then log "FAIL: backup-db exited rc=$_rc"; notify "backup FAILED (rc=$_rc)"; fi' EXIT

usage() {
    cat <<'EOF'
backup-db.sh - paddock DB を durable な場所へ退避する（#265）

使い方:
  scripts/backup-db.sh            # ローカル権威 dir へ退避＋世代管理（既定・ミラー無効）
  scripts/backup-db.sh -h|--help

環境変数:
  PADDOCK_BACKUP_DIR         ローカル権威の退避先（列挙・剪定はここで行う）
                             （既定: ~/paddock-backups）
  PADDOCK_BACKUP_MIRROR_DIR  off-machine ミラー先（ディスク障害対策・オプトイン）。既定は空=無効。
                             実ファイルシステム（外付け/NAS 等）を指定する。iCloud は使わない
                             （launchd 下で剪定が no-op になり溜まるため。#494）
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
MIRROR_DIR="${PADDOCK_BACKUP_MIRROR_DIR:-}"
KEEP="${PADDOCK_BACKUP_KEEP:-14}"
CONTAINER="${PADDOCK_PG_CONTAINER:-paddock-postgres}"
PG_USER="${PADDOCK_PG_USER:-paddock}"
PG_DB="${PADDOCK_PG_DB:-paddock}"

# 権威と同一 dir へのミラーは無意味（cp が same-file エラーになる）なので無効化する。
if [[ "$MIRROR_DIR" == "$BACKUP_DIR" ]]; then
    MIRROR_DIR=""
fi

# KEEP は正整数のみ許可（0 や非整数だと世代管理で作成直後の dump ごと全削除する footgun）。
if ! [[ "$KEEP" =~ ^[1-9][0-9]*$ ]]; then
    echo "PADDOCK_BACKUP_KEEP は正整数である必要がある: $KEEP" >&2
    exit 2
fi

# ここまでで入力検証は完了。以降の非ゼロ終了は実失敗として FAIL 通知の対象にする。
_validated=1

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
_tmp="$final.part"

# container 内 pg_dump（バージョン一致）で full DB を custom-format 退避。stdout をホストファイルへ。
# 一時ファイル(.part)に書き、成功＋非空を確認してから最終名へ mv（中断で壊れた dump を残さない）。
if ! docker exec "$CONTAINER" pg_dump -U "$PG_USER" -d "$PG_DB" -Fc --no-owner --no-privileges > "$_tmp"; then
    echo "pg_dump に失敗（container=$CONTAINER）" >&2
    exit 1
fi
if [[ ! -s "$_tmp" ]]; then
    echo "dump が空（pg_dump は成功したが 0 バイト）" >&2
    exit 1
fi
mv "$_tmp" "$final"
# mv 成功後は一時ファイルが存在しないので _tmp をリセット（EXIT ハンドラで rm しない）。
_tmp=""

size="$(du -h "$final" | cut -f1)"
log "退避完了: $final ($size)"

# off-machine ミラー（best-effort）。cp はパス指定の書き込みなので launchd 下でも効く。失敗しても
# ローカル退避は成功しているので警告に留める（ログで検知する）。
if [[ -n "$MIRROR_DIR" ]]; then
    if mkdir -p "$MIRROR_DIR" && cp -f "$final" "$MIRROR_DIR/$base"; then
        log "ミラー完了: $MIRROR_DIR/$base"
    else
        echo "警告: ミラーに失敗（ローカル退避は成功。$MIRROR_DIR を確認）" >&2
    fi
fi

# 世代管理: 指定 dir を独立に列挙し新しい順に KEEP 個を残して超過分を削除する。権威(ローカル)と
# ミラー(指定時)の両方に同じロジックを適用する（各 dir を独立列挙）。ローカルは常に KEEP に bounded。
# macOS 既定の /bin/bash 3.2（launchd もこれを使う）に mapfile が無いため while-read で読む。
# 列挙は ls -1t のパース（出力パース忌避・空 dir で glob 非展開の罠あり）を避け、find でファイルのみを
# 厳密に収集し、BSD stat（macOS 前提）で mtime を前置して整列する（新しい順）。find -exec ... + は対象
# 0 件なら stat を呼ばず空を返すため、空 dir でもエラーにならない。ソートキーは第1=mtime 数値降順、
# 第2=行残余（=パス。ファイル名に生成時刻 YYYYMMDD-HHMMSS を含む）降順で、同 mtime 時も決定的かつ
# 新しい世代を先頭に固定する（cp 等で mtime が揃った dump でも剪定対象がブレない）。パスに空白を含んでも
# -t' ' + -k2 は第2キーを行末まで一括で見るため分断されない。
prune_dir() {
    local dir="$1" label="$2"
    local files=() f i=0
    while IFS= read -r f; do
        [[ -n "$f" ]] && files+=("$f")
    done < <(
        find "$dir" -maxdepth 1 -type f -name 'paddock-*.dump' -exec stat -f '%m %N' {} + 2>/dev/null \
            | sort -t' ' -k1,1rn -k2r | cut -d' ' -f2-
    )
    if (( ${#files[@]} > KEEP )); then
        for f in "${files[@]}"; do
            if (( i >= KEEP )); then
                rm -f "$f"
                log "古い世代を削除($label): $(basename "$f")"
            fi
            i=$((i + 1))
        done
    fi
    local kept=$(( ${#files[@]} > KEEP ? KEEP : ${#files[@]} ))
    log "保持世代数($label): $kept / KEEP=$KEEP  $dir"
}

prune_dir "$BACKUP_DIR" "権威"
if [[ -n "$MIRROR_DIR" ]]; then
    prune_dir "$MIRROR_DIR" "ミラー"
fi
