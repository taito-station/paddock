#!/usr/bin/env bash
# scripts/lib/pg-container.sh の実行環境判定（#731）と、それを使う backup-db.sh の回帰テスト。
#
# 背景: backup-db.sh / verify-backup-restore.sh は docker exec / docker ps を直に呼んでいたため、
# 開発機の実行環境が colima（docker）から Lima VM 内の rootless nerdctl へ移ってからずっと
# 「コンテナが起動していない」という誤診断で失敗していた（#731）。lima/docker どちらでも動く
# ことと、どちらも無いときは黙って進まず原因を列挙して rc=1 になることをここで固定する。
#
# 本物の limactl / docker / nerdctl / postgres には一切触らない。PATH 差し替えの偽 limactl /
# 偽 docker で
#   - `limactl list --format ...`            … VM の Running/Stopped を模す
#   - `limactl shell <vm> -- nerdctl ps/exec` … nerdctl exec 経由の pg_dump/pg_restore/psql を模す
#   - `docker ps` / `docker exec`             … docker 経由の同上を模す
# を用意する。呼び出し引数はすべて CALL_LOG に記録し、実際に組み立てられたコマンド（-i の有無・
# lima/docker のどちらを叩いたか）をアサートする。
#
# `stat -f '%m %N'`（backup-db.sh の世代管理・prune_dir が使う BSD stat 書式）は GNU coreutils の
# `-f` と意味が異なる（GNU は「ファイルシステム情報」、BSD は「FORMAT 指定」）ため、CI(ubuntu) で
# 実 stat を使うと本番(macOS)と異なる挙動になる。**実 stat は PATH に入れず**、BSD 互換の挙動だけを
# 返す偽 stat を置いて macOS/Linux 両方で同じ経路を通す（backup-db.sh 本体は変更しない・別問題）。
#
# osascript は無害化した偽物に差し替える（実通知を出さない・失敗時のブロックを防ぐ）。
#
# 使い方: bash scripts/test-backup-runtime.sh   （全ケース PASS で exit 0）
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1
REPO_ROOT="$PWD"
LIB="$REPO_ROOT/scripts/lib/pg-container.sh"
BACKUP_SCRIPT="$REPO_ROOT/scripts/backup-db.sh"
VERIFY_SCRIPT="$REPO_ROOT/scripts/verify-backup-restore.sh"

export LANG=C.UTF-8 LC_ALL=C.UTF-8
unset PADDOCK_PG_RUNTIME PADDOCK_LIMA_VM PADDOCK_PG_CONTAINER PADDOCK_PG_USER PADDOCK_PG_DB
unset PADDOCK_BACKUP_DIR PADDOCK_BACKUP_MIRROR_DIR PADDOCK_BACKUP_KEEP PADDOCK_VERIFY_TABLES
# worktree から叩くと GIT_DIR 等が継承されて本物の index を汚す（#645 の実害と同じ罠）。
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE

pass=0
fail=0
ok() { echo "OK  $1"; pass=$((pass + 1)); }
ng() { echo "NG  $1"; shift; [ $# -gt 0 ] && echo "    $*"; fail=$((fail + 1)); }

TESTROOT="$(mktemp -d "${TMPDIR:-/tmp}/paddock-backup-runtime-test.XXXXXX")"
cleanup() { rm -rf "$TESTROOT"; return 0; }
trap cleanup EXIT

# ---- 共通スタブ（実 stat/limactl/docker は置かない。他は本物を symlink） -----------------------
BASE_STUB="$TESTROOT/bin-base"
mkdir -p "$BASE_STUB"
for c in bash env dirname date mkdir mv rm du cut sort find basename grep cp cat; do
    p="$(command -v "$c")" || { echo "前提コマンドが無い: $c" >&2; exit 2; }
    case "$p" in
        /*) ;;
        *) echo "外部コマンドとして解決できない: ${c}（command -v の結果: ${p}）" >&2; exit 2 ;;
    esac
    ln -s "$p" "$BASE_STUB/$c"
done

# BSD 互換の `stat -f '%m %N' file...` だけに応答する偽 stat（GNU/BSD の差異を CI から隠す）。
cat > "$BASE_STUB/stat" <<'EOS'
#!/usr/bin/env bash
# 実 mtime は不要（各ケースの dump は 1 個か、順序を問わない構成にしている）ので、外部
# コマンド（python3 等。pyenv 経由だと更に tr/sed が要るなど環境依存が強い）に頼らず、
# 固定の epoch 値を返す純 bash 実装にする。
set -u
if [ "${1-}" != "-f" ] || [ "${2-}" != "%m %N" ]; then
    echo "stat stub: 未対応の呼び方: $*" >&2
    exit 2
fi
shift 2
for f in "$@"; do
    printf '%s %s\n' "${FAKE_STAT_MTIME:-1700000000}" "$f"
done
EOS
chmod +x "$BASE_STUB/stat"

cat > "$BASE_STUB/osascript" <<'EOS'
#!/usr/bin/env bash
# 通知は無害化する（表示しない・常に成功）。呼ばれたことだけ記録する。
[ -n "${OSASCRIPT_CALL_LOG-}" ] && echo "osascript $*" >> "$OSASCRIPT_CALL_LOG"
exit 0
EOS
chmod +x "$BASE_STUB/osascript"

# 偽 nerdctl 呼び出し（limactl shell <vm> -- nerdctl ... の "nerdctl ..." 部分）に共通で応答する本体。
# CALL_LOG に生 argv を残し、pg_dump/pg_restore/psql/createdb/dropdb/ps を最小限模す。
fake_engine_body() {
    cat <<'EOS'
sub="${1:-}"
case "$sub" in
    ps)
        printf '%s\n' "${FAKE_PS_NAMES:-paddock-postgres}"
        ;;
    exec)
        shift
        stdin_flag=0
        if [ "${1:-}" = "-i" ]; then stdin_flag=1; shift; fi
        container="${1:-}"; shift
        inner="${1:-}"
        case "$inner" in
            pg_dump) printf '%s' "${FAKE_DUMP_BYTES:-FAKE-DUMP-BYTES}" ;;
            pg_restore) cat >/dev/null ;;
            psql) printf '%s\n' "${FAKE_PSQL_OUTPUT:-race_odds_snapshots	3}" ;;
            createdb|dropdb) : ;;
            *) echo "fake exec: 未対応の inner コマンド: $inner" >&2; exit 1 ;;
        esac
        ;;
    *)
        echo "fake engine: 未対応のサブコマンド: $sub" >&2
        exit 1
        ;;
esac
EOS
}

# 偽 limactl: list / shell <vm> -- nerdctl ... に応答する。
cat > "$BASE_STUB/limactl" <<EOS
#!/usr/bin/env bash
set -u
[ -n "\${CALL_LOG-}" ] && echo "limactl \$*" >> "\$CALL_LOG"
cmd="\${1:-}"
case "\$cmd" in
    list)
        printf '%s\n' "\${FAKE_LIMA_STATUS:-Running}"
        exit 0
        ;;
    shell)
        vm="\$2"
        if [ "\${4:-}" != "nerdctl" ]; then
            echo "fake limactl shell: expected nerdctl, got \${4:-}" >&2
            exit 1
        fi
        shift 4
        $(fake_engine_body)
        exit \$?
        ;;
    *)
        echo "fake limactl: 未対応のコマンド: \$cmd" >&2
        exit 1
        ;;
esac
EOS
chmod +x "$BASE_STUB/limactl"

# 偽 docker: ps / exec に応答する。
cat > "$BASE_STUB/docker" <<EOS
#!/usr/bin/env bash
set -u
[ -n "\${CALL_LOG-}" ] && echo "docker \$*" >> "\$CALL_LOG"
$(fake_engine_body)
exit \$?
EOS
chmod +x "$BASE_STUB/docker"

# ---- ヘルパ -----------------------------------------------------------------
# stub 一式を case 専用ディレクトリへコピーし、必要なコマンドだけを残す（symlink → 実体コピーで
# 個別 rm しても他ケースに影響しない構成にする）。
new_stub() {
    local d="$1"; shift
    mkdir -p "$d"
    local c
    for c in "$@"; do
        cp -P "$BASE_STUB/$c" "$d/$c"
    done
    printf '%s' "$d"
}
case_dir() { local d="$TESTROOT/case-$1"; mkdir -p "$d"; printf '%s' "$d"; }

# resolve_driver.sh: pg_container_resolve_runtime を呼び、結果を1行で出す小さいドライバ。
DRIVER="$TESTROOT/resolve_driver.sh"
cat > "$DRIVER" <<'EOS'
#!/usr/bin/env bash
source "$LIB_PATH"
pg_container_resolve_runtime "${DRIVER_CONTAINER:-paddock-postgres}"
rc=$?
echo "RC=$rc RUNTIME=${PG_RUNTIME:-<unset>} VM=${PG_RUNTIME_VM:-<unset>}"
exit "$rc"
EOS

# require_running_driver.sh: resolve → require_running まで通す（起動確認・ヒント文言の検証用）。
REQUIRE_DRIVER="$TESTROOT/require_driver.sh"
cat > "$REQUIRE_DRIVER" <<'EOS'
#!/usr/bin/env bash
source "$LIB_PATH"
pg_container_require_running "${DRIVER_CONTAINER:-paddock-postgres}"
rc=$?
echo "RC=$rc RUNTIME=${PG_RUNTIME:-<unset>}"
exit "$rc"
EOS

# exec_driver.sh: resolve → pg_container_exec の -i 有無を確認する用。
EXEC_DRIVER="$TESTROOT/exec_driver.sh"
cat > "$EXEC_DRIVER" <<'EOS'
#!/usr/bin/env bash
source "$LIB_PATH"
pg_container_resolve_runtime "${DRIVER_CONTAINER:-paddock-postgres}" || exit 9
if [ "${DRIVER_STDIN:-0}" = "1" ]; then
    pg_container_exec -i "${DRIVER_CONTAINER:-paddock-postgres}" pg_dump
else
    pg_container_exec "${DRIVER_CONTAINER:-paddock-postgres}" pg_dump
fi
EOS

echo "=== pg_container_resolve_runtime（auto 判定） ==="

# --- 1. auto: limactl があり VM Running → lima ---
d="$(case_dir auto-lima)"; stub="$(new_stub "$d/bin" bash env limactl)"
out="$(env -i PATH="$stub" LIB_PATH="$LIB" FAKE_LIMA_STATUS="Running" \
    bash "$DRIVER" 2>&1)"; rc=$?
if [ "$rc" -eq 0 ] && grep -q '^RC=0 RUNTIME=lima VM=paddock$' <<<"$out"; then
    ok "auto: limactl+VM Running → lima"
else
    ng "auto: limactl+VM Running → lima" "rc=$rc / $out"
fi

# --- 2. auto: limactl 無し・docker にコンテナあり → docker ---
d="$(case_dir auto-docker)"; stub="$(new_stub "$d/bin" bash env grep docker)"
out="$(env -i PATH="$stub" LIB_PATH="$LIB" FAKE_PS_NAMES="paddock-postgres" \
    bash "$DRIVER" 2>&1)"; rc=$?
if [ "$rc" -eq 0 ] && grep -q '^RC=0 RUNTIME=docker VM=<unset>$' <<<"$out"; then
    ok "auto: limactl 無し・docker にコンテナあり → docker"
else
    ng "auto: limactl 無し・docker にコンテナあり → docker" "rc=$rc / $out"
fi

# --- 3. auto: limactl あるが VM Stopped、docker にコンテナあり → docker にフォールバック ---
d="$(case_dir auto-fallback)"; stub="$(new_stub "$d/bin" bash env grep limactl docker)"
out="$(env -i PATH="$stub" LIB_PATH="$LIB" FAKE_LIMA_STATUS="Stopped" FAKE_PS_NAMES="paddock-postgres" \
    bash "$DRIVER" 2>&1)"; rc=$?
if [ "$rc" -eq 0 ] && grep -q '^RC=0 RUNTIME=docker VM=<unset>$' <<<"$out"; then
    ok "auto: VM Stopped → docker へフォールバック"
else
    ng "auto: VM Stopped → docker へフォールバック" "rc=$rc / $out"
fi

# --- 4. auto: 両方使える → lima を優先する ---
d="$(case_dir auto-both)"; stub="$(new_stub "$d/bin" bash env limactl docker)"
out="$(env -i PATH="$stub" LIB_PATH="$LIB" FAKE_LIMA_STATUS="Running" FAKE_PS_NAMES="paddock-postgres" \
    bash "$DRIVER" 2>&1)"; rc=$?
if [ "$rc" -eq 0 ] && grep -q '^RC=0 RUNTIME=lima VM=paddock$' <<<"$out"; then
    ok "auto: lima と docker 両方使える → lima を優先"
else
    ng "auto: lima と docker 両方使える → lima を優先" "rc=$rc / $out"
fi

# --- 5. auto: どちらも無い/使えない → rc=1 で原因を列挙する ---
d="$(case_dir auto-none)"; stub="$(new_stub "$d/bin" bash env)"
out="$(env -i PATH="$stub" LIB_PATH="$LIB" bash "$DRIVER" 2>&1)"; rc=$?
if [ "$rc" -eq 1 ] && grep -q 'limactl: 見つからない' <<<"$out" && grep -q 'docker: 見つからない' <<<"$out"; then
    ok "auto: limactl/docker どちらも無い → rc=1 で列挙"
else
    ng "auto: limactl/docker どちらも無い → rc=1 で列挙" "rc=$rc / $out"
fi

# --- 6. auto: limactl はあるが VM 未作成・docker のコンテナも見えない → rc=1 で状態を列挙 ---
d="$(case_dir auto-unreachable)"; stub="$(new_stub "$d/bin" bash env grep limactl docker)"
out="$(env -i PATH="$stub" LIB_PATH="$LIB" FAKE_LIMA_STATUS="Stopped" FAKE_PS_NAMES="other-container" \
    bash "$DRIVER" 2>&1)"; rc=$?
if [ "$rc" -eq 1 ] && grep -q 'limactl: 見つかった' <<<"$out" && grep -q 'docker: 見つかった' <<<"$out"; then
    ok "auto: 両方見つかるが起動確認できない → rc=1 で状態を列挙"
else
    ng "auto: 両方見つかるが起動確認できない → rc=1 で状態を列挙" "rc=$rc / $out"
fi

echo "=== PADDOCK_PG_RUNTIME 明示指定 ==="

# --- 7. lima 明示: VM が Stopped でも判定は lima に固定する（起動確認は require_running 側の責務） ---
d="$(case_dir explicit-lima)"; stub="$(new_stub "$d/bin" bash env limactl docker)"
out="$(env -i PATH="$stub" LIB_PATH="$LIB" PADDOCK_PG_RUNTIME="lima" \
    FAKE_LIMA_STATUS="Stopped" FAKE_PS_NAMES="paddock-postgres" bash "$DRIVER" 2>&1)"; rc=$?
if [ "$rc" -eq 0 ] && grep -q '^RC=0 RUNTIME=lima VM=paddock$' <<<"$out"; then
    ok "明示 PADDOCK_PG_RUNTIME=lima → docker が使えても lima 固定"
else
    ng "明示 PADDOCK_PG_RUNTIME=lima → docker が使えても lima 固定" "rc=$rc / $out"
fi

# --- 8. docker 明示: lima が Running でも docker に固定する ---
d="$(case_dir explicit-docker)"; stub="$(new_stub "$d/bin" bash env limactl docker)"
out="$(env -i PATH="$stub" LIB_PATH="$LIB" PADDOCK_PG_RUNTIME="docker" \
    FAKE_LIMA_STATUS="Running" bash "$DRIVER" 2>&1)"; rc=$?
if [ "$rc" -eq 0 ] && grep -q '^RC=0 RUNTIME=docker VM=<unset>$' <<<"$out"; then
    ok "明示 PADDOCK_PG_RUNTIME=docker → lima が使えても docker 固定"
else
    ng "明示 PADDOCK_PG_RUNTIME=docker → lima が使えても docker 固定" "rc=$rc / $out"
fi

# --- 9. lima 明示だが limactl が無い → rc=1 ---
d="$(case_dir explicit-lima-missing)"; stub="$(new_stub "$d/bin" bash env)"
out="$(env -i PATH="$stub" LIB_PATH="$LIB" PADDOCK_PG_RUNTIME="lima" bash "$DRIVER" 2>&1)"; rc=$?
if [ "$rc" -eq 1 ] && grep -q 'limactl が見つからない' <<<"$out"; then
    ok "明示 PADDOCK_PG_RUNTIME=lima だが limactl 無し → rc=1"
else
    ng "明示 PADDOCK_PG_RUNTIME=lima だが limactl 無し → rc=1" "rc=$rc / $out"
fi

# --- 10. 不正な値 → rc=1 ---
d="$(case_dir explicit-invalid)"; stub="$(new_stub "$d/bin" bash env)"
out="$(env -i PATH="$stub" LIB_PATH="$LIB" PADDOCK_PG_RUNTIME="podman" bash "$DRIVER" 2>&1)"; rc=$?
if [ "$rc" -eq 1 ] && grep -q 'auto|lima|docker' <<<"$out"; then
    ok "PADDOCK_PG_RUNTIME=podman（不正値） → rc=1"
else
    ng "PADDOCK_PG_RUNTIME=podman（不正値） → rc=1" "rc=$rc / $out"
fi

echo "=== pg_container_require_running（起動確認・ヒント文言） ==="

# --- 11. lima 経路でコンテナが ps に見えない → rc=1 で nerdctl compose のヒントを出す ---
d="$(case_dir require-lima-down)"; stub="$(new_stub "$d/bin" bash env limactl)"
out="$(env -i PATH="$stub" LIB_PATH="$LIB" FAKE_LIMA_STATUS="Running" FAKE_PS_NAMES="other" \
    bash "$REQUIRE_DRIVER" 2>&1)"; rc=$?
if [ "$rc" -eq 1 ] && grep -q 'nerdctl compose' <<<"$out"; then
    ok "lima 経路・コンテナ未起動 → nerdctl compose のヒントで rc=1"
else
    ng "lima 経路・コンテナ未起動 → nerdctl compose のヒントで rc=1" "rc=$rc / $out"
fi

# --- 12. docker 経路でコンテナが ps に見えない → rc=1 で docker compose のヒントを出す ---
# auto 判定は docker 選択の条件そのものが「対象コンテナが docker ps に見える」ことなので、
# auto では「docker が選ばれたのにコンテナが落ちている」状態には到達できない。
# PADDOCK_PG_RUNTIME=docker を明示してこの状態を作る（explicit-docker と同じ理由）。
d="$(case_dir require-docker-down)"; stub="$(new_stub "$d/bin" bash env grep docker)"
out="$(env -i PATH="$stub" LIB_PATH="$LIB" PADDOCK_PG_RUNTIME="docker" FAKE_PS_NAMES="other" \
    bash "$REQUIRE_DRIVER" 2>&1)"; rc=$?
if [ "$rc" -eq 1 ] && grep -q 'docker compose' <<<"$out" && ! grep -q 'nerdctl' <<<"$out"; then
    ok "docker 経路・コンテナ未起動 → docker compose のヒントで rc=1"
else
    ng "docker 経路・コンテナ未起動 → docker compose のヒントで rc=1" "rc=$rc / $out"
fi

echo "=== pg_container_exec（-i の伝播） ==="

# --- 13. -i 指定時は nerdctl exec に -i が付く ---
d="$(case_dir exec-stdin)"; stub="$(new_stub "$d/bin" bash env limactl)"
log="$d/call.log"
env -i PATH="$stub" LIB_PATH="$LIB" CALL_LOG="$log" FAKE_LIMA_STATUS="Running" DRIVER_STDIN=1 \
    bash "$EXEC_DRIVER" >/dev/null 2>&1
if grep -qE '^limactl shell paddock -- nerdctl exec -i paddock-postgres pg_dump$' "$log" 2>/dev/null; then
    ok "pg_container_exec -i → nerdctl exec に -i が伝播する"
else
    ng "pg_container_exec -i → nerdctl exec に -i が伝播する" "$(cat "$log" 2>/dev/null)"
fi

# --- 14. -i 未指定時は nerdctl exec に -i が付かない ---
d="$(case_dir exec-nostdin)"; stub="$(new_stub "$d/bin" bash env limactl)"
log="$d/call.log"
env -i PATH="$stub" LIB_PATH="$LIB" CALL_LOG="$log" FAKE_LIMA_STATUS="Running" DRIVER_STDIN=0 \
    bash "$EXEC_DRIVER" >/dev/null 2>&1
if grep -qE '^limactl shell paddock -- nerdctl exec paddock-postgres pg_dump$' "$log" 2>/dev/null; then
    ok "pg_container_exec（-i 無し） → nerdctl exec に -i が付かない"
else
    ng "pg_container_exec（-i 無し） → nerdctl exec に -i が付かない" "$(cat "$log" 2>/dev/null)"
fi

echo "=== backup-db.sh 本体（lima/docker 経路の end-to-end） ==="

# --- 15. auto・lima 経路: dump と .rowcounts が作られ rc=0 ---
d="$(case_dir e2e-lima)"; stub="$(new_stub "$d/bin" bash env dirname date mkdir mv rm du cut sort find basename grep cat stat osascript limactl)"
backup_dir="$d/backups"
out="$(env -i PATH="$stub" HOME="$d/home" \
    PADDOCK_BACKUP_DIR="$backup_dir" PADDOCK_VERIFY_TABLES="race_odds_snapshots" \
    FAKE_LIMA_STATUS="Running" FAKE_DUMP_BYTES="FAKE-DUMP-BYTES" \
    FAKE_PSQL_OUTPUT="$(printf 'race_odds_snapshots\t3')" \
    bash "$BACKUP_SCRIPT" 2>&1)"; rc=$?
dump="$(find "$backup_dir" -maxdepth 1 -name 'paddock-*.dump' 2>/dev/null | head -1)"
if [ "$rc" -eq 0 ] && [ -n "$dump" ] && [ -s "$dump" ] && [ -f "$dump.rowcounts" ] \
    && grep -qxF $'race_odds_snapshots\t3' "$dump.rowcounts"; then
    ok "backup-db.sh: auto・lima 経路 → dump + .rowcounts 生成・rc=0"
else
    ng "backup-db.sh: auto・lima 経路 → dump + .rowcounts 生成・rc=0" "rc=$rc dump=$dump / $out"
fi

# --- 16. auto・docker 経路（limactl 無し）: dump と .rowcounts が作られ rc=0 ---
d="$(case_dir e2e-docker)"; stub="$(new_stub "$d/bin" bash env dirname date mkdir mv rm du cut sort find basename grep cat stat osascript docker)"
backup_dir="$d/backups"
out="$(env -i PATH="$stub" HOME="$d/home" \
    PADDOCK_BACKUP_DIR="$backup_dir" PADDOCK_VERIFY_TABLES="race_odds_snapshots" \
    FAKE_PS_NAMES="paddock-postgres" FAKE_DUMP_BYTES="FAKE-DUMP-BYTES" \
    FAKE_PSQL_OUTPUT="$(printf 'race_odds_snapshots\t3')" \
    bash "$BACKUP_SCRIPT" 2>&1)"; rc=$?
dump="$(find "$backup_dir" -maxdepth 1 -name 'paddock-*.dump' 2>/dev/null | head -1)"
if [ "$rc" -eq 0 ] && [ -n "$dump" ] && [ -s "$dump" ] && [ -f "$dump.rowcounts" ] \
    && grep -qxF $'race_odds_snapshots\t3' "$dump.rowcounts"; then
    ok "backup-db.sh: auto・docker 経路（limactl 無し） → dump + .rowcounts 生成・rc=0"
else
    ng "backup-db.sh: auto・docker 経路（limactl 無し） → dump + .rowcounts 生成・rc=0" "rc=$rc dump=$dump / $out"
fi

# --- 17. 実行環境が見つからない → rc=1・dump は作られない ---
d="$(case_dir e2e-none)"; stub="$(new_stub "$d/bin" bash env dirname date mkdir mv rm du cut sort find basename grep stat osascript)"
backup_dir="$d/backups"
out="$(env -i PATH="$stub" HOME="$d/home" PADDOCK_BACKUP_DIR="$backup_dir" \
    bash "$BACKUP_SCRIPT" 2>&1)"; rc=$?
dump_count="$(find "$backup_dir" -maxdepth 1 -name 'paddock-*.dump' 2>/dev/null | wc -l | tr -d ' ')"
if [ "$rc" -eq 1 ] && [ "${dump_count:-0}" -eq 0 ] && grep -q '見つからない' <<<"$out"; then
    ok "backup-db.sh: 実行環境が見つからない → rc=1・dump 未生成"
else
    ng "backup-db.sh: 実行環境が見つからない → rc=1・dump 未生成" "rc=$rc dump_count=$dump_count / $out"
fi

echo "=== verify-backup-restore.sh 本体（lima 経路の end-to-end） ==="

# dump 選択（find -exec stat ...）とサイドカー突合の両方を実際に踏ませる。dump の中身は
# pg_restore --list/pg_restore がどちらも fake で中身を読まないため任意のバイト列でよい。
mk_dump_fixture() {
    local dir="$1" rows="$2"
    mkdir -p "$dir"
    local dump="$dir/paddock-20260101-000000.dump"
    printf 'FAKE-DUMP-BYTES' > "$dump"
    printf '%s\n' "$rows" > "$dump.rowcounts"
    printf '%s' "$dump"
}

# --- 18. lima 経路: サイドカー記録値と scratch 復元行数が一致 → SUCCESS で rc=0 ---
d="$(case_dir verify-lima-ok)"; stub="$(new_stub "$d/bin" bash env dirname date mkdir rm du cut sort find basename grep cat stat osascript limactl)"
backup_dir="$d/backups"
mk_dump_fixture "$backup_dir" $'race_odds_snapshots\t3' >/dev/null
out="$(env -i PATH="$stub" HOME="$d/home" PADDOCK_BACKUP_DIR="$backup_dir" \
    FAKE_LIMA_STATUS="Running" FAKE_PSQL_OUTPUT="3" \
    bash "$VERIFY_SCRIPT" 2>&1)"; rc=$?
if [ "$rc" -eq 0 ] && grep -q 'SUCCESS:' <<<"$out"; then
    ok "verify-backup-restore.sh: lima 経路・行数一致 → SUCCESS・rc=0"
else
    ng "verify-backup-restore.sh: lima 経路・行数一致 → SUCCESS・rc=0" "rc=$rc / $out"
fi

# --- 19. lima 経路: サイドカー記録値と scratch 復元行数が不一致 → FAIL で rc=1（scratch は削除する） ---
d="$(case_dir verify-lima-mismatch)"; stub="$(new_stub "$d/bin" bash env dirname date mkdir rm du cut sort find basename grep cat stat osascript limactl)"
backup_dir="$d/backups"
mk_dump_fixture "$backup_dir" $'race_odds_snapshots\t3' >/dev/null
log="$d/call.log"
out="$(env -i PATH="$stub" HOME="$d/home" PADDOCK_BACKUP_DIR="$backup_dir" CALL_LOG="$log" \
    FAKE_LIMA_STATUS="Running" FAKE_PSQL_OUTPUT="5" \
    bash "$VERIFY_SCRIPT" 2>&1)"; rc=$?
if [ "$rc" -eq 1 ] && grep -q 'MISMATCH' <<<"$out" && grep -q 'dropdb' "$log" 2>/dev/null; then
    ok "verify-backup-restore.sh: lima 経路・行数不一致 → FAIL・rc=1・scratch DB は削除される"
else
    ng "verify-backup-restore.sh: lima 経路・行数不一致 → FAIL・rc=1・scratch DB は削除される" "rc=$rc / $out"
fi

echo
echo "PASS=$pass FAIL=$fail"
[ "$fail" -eq 0 ] || exit 1
exit 0
