#!/usr/bin/env bash
# keep_awake.sh の抑止窓の追従（#585）と lock パス（#643）の回帰テスト。
#
# **このスクリプトは開催日にしか本番を踏まない**（launchd の keep-awake は開催日の朝に install
# する運用）。壊れても次の開催まで気づけないので、判断分岐をここで固定する。
#
# 実 caffeinate は起動しない。PATH 差し替えのスタブで
#   - `psql`       … 最終 post_time を返す（DB に触らない）
#   - `caffeinate` … 引数を記録して `sleep` に化ける（実 pid が生きるので kill -0 / kill は本物）
#   - `ps`         … `-p PID -o comm=` に "caffeinate" を返す（スタブは shebang 実行なので
#                     実 ps だと comm が bash になり、稼働中判定が成立しないため）
# を用意する。lock は `PADDOCK_KEEP_AWAKE_LOCK_DIR` でテスト専用ディレクトリへ逃がし、
# 実運用の /tmp/paddock-keep-awake-$(id -u).lock.d には一切触らない（稼働中の本物を殺さない）。
#
# 使い方: bash scripts/test-keep-awake.sh   （全ケース PASS で exit 0）
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1
REPO_ROOT="$PWD"
# ロケール非依存で全角混じりメッセージを扱う。呼び出し元の PADDOCK_* 汚染を避ける。
export LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8
unset PADDOCK_DB_URL PADDOCK_KEEP_AWAKE_LOCK_DIR WORKDIR
# worktree から叩くと GIT_DIR 等が継承されて本物の index を汚す（#645 の実害）。
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE

pass=0
fail=0
ok()  { echo "OK  $1"; pass=$((pass + 1)); }
ng()  { echo "NG  $1"; shift; [ $# -gt 0 ] && echo "    $*"; fail=$((fail + 1)); }
# caffeinate を起動したか。**スタブが書くログではなく keep_awake.sh 自身の出力で判定する**
# ——スタブは nohup された背景プロセスなので、書き込みが親の終了に間に合わず競合する。
# 親が同期的に出す「起動（pid ...）」行なら取りこぼさない。
started() { grep -q '起動（pid' <<<"$1"; }

TESTROOT="$(mktemp -d "${TMPDIR:-/tmp}/paddock-keep-awake-test.XXXXXX")"
cleanup() {
  # 取り残した偽 caffeinate（sleep）を確実に始末する。テストが実機に sleep を残さない。
  if [ -f "$TESTROOT/spawned" ]; then
    while read -r p; do [ -n "$p" ] && kill "$p" 2>/dev/null; done < "$TESTROOT/spawned"
  fi
  rm -rf "$TESTROOT"
  return 0
}
trap cleanup EXIT

# ---- スタブ -----------------------------------------------------------------
STUB="$TESTROOT/bin"
mkdir -p "$STUB"

cat > "$STUB/psql" <<'EOS'
#!/usr/bin/env bash
# keep_awake.sh は `psql "$URL" -tA -c "SELECT MAX(post_time) ..."` の形で呼ぶ。
# DB へは行かず、FAKE_LAST_POST をそのまま返す（空なら「開催外」を模す）。
printf '%s\n' "${FAKE_LAST_POST-}"
EOS

cat > "$STUB/caffeinate" <<'EOS'
#!/usr/bin/env bash
# 引数を記録してから sleep に化ける。実 pid が生きるので kill -0 / kill は本物が効く。
printf '%s\n' "$*" >> "${FAKE_CAFFEINATE_LOG:?}"
printf '%s\n' "$$" >> "${FAKE_SPAWNED_LOG:?}"
exec sleep 300
EOS

cat > "$STUB/ps" <<'EOS'
#!/usr/bin/env bash
# `ps -p PID -o comm=` だけを模す。生きている pid には "caffeinate" を返す。
# FAKE_PS_ALIEN_PID に指定した pid だけは別プロセス名を返す（PID 再利用の誤判定を潰す分岐用）。
pid=""
while [ $# -gt 0 ]; do
  case "$1" in
    -p) pid="${2-}"; shift 2 ;;
    -o) shift 2 ;;
    *)  shift ;;
  esac
done
if [ -n "${FAKE_PS_ALIEN_PID-}" ] && [ "$pid" = "${FAKE_PS_ALIEN_PID}" ]; then
  echo "Dock"; exit 0
fi
kill -0 "$pid" 2>/dev/null && echo "caffeinate"
exit 0
EOS

chmod +x "$STUB/psql" "$STUB/caffeinate" "$STUB/ps"

# ---- ヘルパ -----------------------------------------------------------------
# run_keep_awake <lockdir> <--at HH:MM ...> : スタブ PATH で keep_awake.sh を走らせ、
# 標準出力＋標準エラーを返す。FAKE_* は呼び出し側が export 済みである前提。
run_keep_awake() {
  local lockdir="$1"; shift
  PATH="$STUB:$PATH" \
  PADDOCK_KEEP_AWAKE_LOCK_DIR="$lockdir" \
  WORKDIR="$TESTROOT/work" \
  PADDOCK_DB_URL="postgres://p:p@127.0.0.1:1/paddock" \
    bash "$REPO_ROOT/scripts/predict-check/keep_awake.sh" "$@" 2>&1
}

# 偽 caffeinate を 1 本起動して pid を返す（lock の「稼働中」を作るため）。
# **fd を必ず /dev/null へ落とす**: この関数は `$( )` の中で呼ばれるので、背景プロセスが
# 置換の stdout パイプを握ったままだと `$( )` が sleep の終了まで戻らずテストが固まる。
spawn_fake_caffeinate() {
  sleep 300 >/dev/null 2>&1 &
  local p=$!
  echo "$p" >> "$TESTROOT/spawned"
  echo "$p"
}

export FAKE_CAFFEINATE_LOG="$TESTROOT/caffeinate.args"
export FAKE_SPAWNED_LOG="$TESTROOT/spawned"
: > "$FAKE_CAFFEINATE_LOG"
: > "$FAKE_SPAWNED_LOG"
export FAKE_LAST_POST="18:30"

# 各ケースは専用 lockdir を使う（相互汚染を避ける）。
case_dir() { local d="$TESTROOT/case-$1"; mkdir -p "$d"; echo "$d/lock.d"; }

echo "=== #585 抑止窓の追従 ==="

# --- 1. 記録された終了時刻が必要窓を満たす → 据え置き（起動しない） ---
L="$(case_dir keep)"; mkdir -p "$L"
caf="$(spawn_fake_caffeinate)"
echo "$caf" > "$L/pid"
echo "$(($(date +%s) + 86400))" > "$L/end"   # 十分先＝延長不要
out="$(run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if ! started "$out" && grep -q '延長不要' <<<"$out"; then
  ok "end が必要窓を満たすなら caffeinate を起動しない（据え置き）"
else
  ng "end が必要窓を満たすなら据え置き" "out=$out"
fi
kill -0 "$caf" 2>/dev/null && ok "据え置き時は旧 caffeinate を殺さない" \
  || ng "据え置き時は旧 caffeinate を殺さない" "pid ${caf} が消えた"

# --- 2. 記録が必要窓に足りない → 延長（新を起動してから旧を落とす） ---
L="$(case_dir extend)"; mkdir -p "$L"
caf="$(spawn_fake_caffeinate)"
echo "$caf" > "$L/pid"
echo "$(($(date +%s) + 60))" > "$L/end"      # 1 分後まで＝18:30 まで足りない
out="$(run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if started "$out" && grep -q '抑止窓を延長する' <<<"$out"; then
  ok "end が足りなければ延長する（新しい caffeinate を起動）"
else
  ng "end が足りなければ延長する" "out=$out"
fi
if grep -q '旧 caffeinate を停止' <<<"$out"; then
  ok "延長時は旧 caffeinate を停止する"
else
  ng "延長時は旧 caffeinate を停止する" "out=$out"
fi
# **順序の固定**: 新しい pid が lock に書かれてから旧が落ちる。ログの並びで検証する
# （kill→start の順に変えるとこの並びが崩れる＝抑止空白が生まれる実装に戻ったことを検知）。
start_line="$(grep -n '起動（pid' <<<"$out" | head -1 | cut -d: -f1)"
kill_line="$(grep -n '旧 caffeinate を停止' <<<"$out" | head -1 | cut -d: -f1)"
if [ -n "$start_line" ] && [ -n "$kill_line" ] && [ "$start_line" -lt "$kill_line" ]; then
  ok "起動→停止の順序（抑止の空白を作らない）"
else
  ng "起動→停止の順序" "start=$start_line kill=$kill_line out=$out"
fi
new_pid="$(cat "$L/pid" 2>/dev/null || echo '')"
new_end="$(cat "$L/end" 2>/dev/null || echo '')"
if [ -n "$new_pid" ] && [ "$new_pid" != "$caf" ] && [ -n "$new_end" ]; then
  ok "延長後の lock に新しい pid と end が記録される"
else
  ng "延長後の lock に新しい pid と end" "pid=${new_pid}（旧=${caf}） end=${new_end}"
fi

# --- 3. end が無い（旧形式 lock）→ 安全側に倒して張り直す ---
L="$(case_dir legacy_format)"; mkdir -p "$L"
caf="$(spawn_fake_caffeinate)"
echo "$caf" > "$L/pid"                        # end は書かない
out="$(run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if started "$out" && grep -q '旧形式' <<<"$out"; then
  ok "end 未記入の lock は安全側に倒して張り直す"
else
  ng "end 未記入の lock は張り直す" "out=$out"
fi

# --- 4. pid 未記入かつ新しい lock → 「起動中」で据え置き（既存 self-heal を壊していない） ---
L="$(case_dir starting)"; mkdir -p "$L"
out="$(run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if ! started "$out" && grep -q '別プロセスが起動中' <<<"$out"; then
  ok "pid 未記入かつ新しい lock は起動中とみなす（STARTUP_GRACE_MIN の self-heal 不変）"
else
  ng "pid 未記入かつ新しい lock は起動中とみなす" "out=$out"
fi

# --- 5. pid が死んでいる stale lock → 取り直す ---
L="$(case_dir stale)"; mkdir -p "$L"
dead="$(spawn_fake_caffeinate)"; kill "$dead" 2>/dev/null; wait "$dead" 2>/dev/null
echo "$dead" > "$L/pid"
echo "$(($(date +%s) + 86400))" > "$L/end"   # end は十分先でも pid が死んでいれば取り直す
out="$(run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if started "$out" && grep -q 'stale lock' <<<"$out"; then
  ok "pid が死んだ lock は end が新しくても取り直す"
else
  ng "pid が死んだ lock は取り直す" "out=$out"
fi

# --- 6. pid は生きているが caffeinate でない（PID 再利用）→ stale 扱い ---
L="$(case_dir alien)"; mkdir -p "$L"
alien="$(spawn_fake_caffeinate)"
echo "$alien" > "$L/pid"
echo "$(($(date +%s) + 86400))" > "$L/end"
out="$(FAKE_PS_ALIEN_PID="$alien" run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if started "$out" && grep -q 'stale lock' <<<"$out"; then
  ok "pid が caffeinate でなければ stale 扱い（comm 照合が効いている）"
else
  ng "pid が caffeinate でなければ stale 扱い" "out=$out"
fi

echo "=== #643 lock パス ==="

# --- 7. 旧パスに生きた caffeinate → 引き継いで張り直し、旧ディレクトリは消える ---
# 旧パスはハードコード（/tmp/paddock-keep-awake.lock.d）なので、**実運用と衝突しないよう
# 既に存在するときはこのケースを skip する**（本物の caffeinate を横取りしない）。
LEGACY="/tmp/paddock-keep-awake.lock.d"
if [ -e "$LEGACY" ]; then
  echo "SKIP 旧 lock 引き継ぎ（${LEGACY} が実在＝実運用中の可能性。横取りしない）"
else
  L="$(case_dir legacy_path)"
  mkdir -p "$LEGACY"
  caf="$(spawn_fake_caffeinate)"
  echo "$caf" > "$LEGACY/pid"
  out="$(run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
  if grep -q '旧 lock パスの caffeinate を引き継ぐ' <<<"$out" && started "$out"; then
    ok "旧 lock パスの caffeinate を引き継いで張り直す"
  else
    ng "旧 lock パスの caffeinate を引き継ぐ" "out=$out"
  fi
  if grep -q '旧 caffeinate を停止' <<<"$out"; then
    ok "引き継いだ旧 caffeinate を停止する（二重抑止を残さない）"
  else
    ng "引き継いだ旧 caffeinate を停止する" "out=$out"
  fi
  if [ ! -d "$LEGACY" ]; then
    ok "旧 lock ディレクトリは移行後に消える"
  else
    ng "旧 lock ディレクトリは移行後に消える" "${LEGACY} が残っている"
    rm -rf "$LEGACY"
  fi
fi

# --- 8. 既定 lock パスが uid スコープで、作成側と削除側が同じ式を持つ ---
# #643 の要件「作成側と削除側の両方を同時に直す」の機械的な担保。片方だけ変えると
# uninstall が caffeinate を止められなくなる（＝抑止が居座る）。
EXPECT_EXPR='LOCK_DIR="${PADDOCK_KEEP_AWAKE_LOCK_DIR:-/tmp/paddock-keep-awake-$(id -u).lock.d}"'
creator="$(grep -c -F "$EXPECT_EXPR" "$REPO_ROOT/scripts/predict-check/keep_awake.sh" || true)"
remover="$(grep -c -F "$EXPECT_EXPR" "$REPO_ROOT/deployments/launchd/uninstall.sh" || true)"
if [ "$creator" -ge 1 ] && [ "$remover" -ge 1 ]; then
  ok "作成側と削除側が同一の uid スコープ lock パス式を持つ"
else
  ng "作成側と削除側が同一の lock パス式を持つ" "keep_awake.sh=${creator} uninstall.sh=${remover}"
fi
# 旧パス（uid 無し）を lock として使い続けていないこと。
# 行頭アンカー必須: LEGACY_LOCK_DIR="/tmp/paddock-keep-awake.lock.d"（移行用の参照）に
# 部分一致してしまうため。見たいのは「LOCK_DIR そのものが旧パスか」。
if grep -q '^LOCK_DIR="/tmp/paddock-keep-awake.lock.d"' "$REPO_ROOT/scripts/predict-check/keep_awake.sh" \
   || grep -q '^LOCK_DIR="/tmp/paddock-keep-awake.lock.d"' "$REPO_ROOT/deployments/launchd/uninstall.sh"; then
  ng "旧 lock パスを LOCK_DIR に使っていない" "uid 無しの固定パスが残っている"
else
  ok "旧 lock パスを LOCK_DIR に使っていない"
fi

echo
echo "=== 合計: PASS=${pass} FAIL=${fail} ==="
[ "$fail" -eq 0 ]
