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
# を用意する。lock は `PADDOCK_KEEP_AWAKE_LOCK_DIR` / `PADDOCK_KEEP_AWAKE_LEGACY_LOCK_DIR` の
# **両方**をテスト専用ディレクトリへ逃がし、実運用の /tmp/paddock-keep-awake*.lock.d には一切
# 触らない（稼働中の本物を殺さない）。旧パス側の注入を忘れると、移行処理は全ケースで無条件に
# 走るので、実運用の旧 lock を読んで本物の caffeinate を kill しうる。
#
# 使い方: bash scripts/test-keep-awake.sh   （全ケース PASS で exit 0）
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1
REPO_ROOT="$PWD"
# ロケール非依存で全角混じりメッセージを扱う。呼び出し元の PADDOCK_* 汚染を避ける。
export LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8
unset PADDOCK_DB_URL PADDOCK_KEEP_AWAKE_LOCK_DIR PADDOCK_KEEP_AWAKE_LEGACY_LOCK_DIR WORKDIR
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
  PADDOCK_KEEP_AWAKE_LEGACY_LOCK_DIR="${TEST_LEGACY_DIR:-$TESTROOT/absent-legacy.lock.d}" \
  WORKDIR="$TESTROOT/work" \
  PADDOCK_DB_URL="postgres://p:p@127.0.0.1:1/paddock" \
    bash "$REPO_ROOT/scripts/predict-check/keep_awake.sh" "$@" 2>&1
}

# uninstall.sh を同じスタブ PATH で走らせる。plist は存在しないので「未インストール」を出して
# lock の片付けだけ行う経路になる。
run_uninstall() {
  local lockdir="$1" legacydir="$2"
  PATH="$STUB:$PATH" \
  PADDOCK_KEEP_AWAKE_LOCK_DIR="$lockdir" \
  PADDOCK_KEEP_AWAKE_LEGACY_LOCK_DIR="$legacydir" \
    bash "$REPO_ROOT/deployments/launchd/uninstall.sh" 2>&1
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
if kill -0 "$caf" 2>/dev/null; then
  ok "据え置き時は旧 caffeinate を殺さない"
else
  ng "据え置き時は旧 caffeinate を殺さない" "pid ${caf} が消えた"
fi

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

# --- 2b. 同じ post_time なら 2 回目は必ず据え置き（判定が実行時刻の秒針で揺れないこと） ---
# END_EPOCH を分境界へ丸めないと、end に実行時刻の秒針が乗る（SECS は分粒度なので
# end = 真の終了時刻 + 秒針）。すると次サイクルの `cur_end >= END_EPOCH` が
# 「前回の秒針 >= 今回の秒針」に退化し、**post_time が変わらなくても延長し続ける**。
# 秒針は必ず進むので、丸めを外すとこのケースは決定的に落ちる（＝R-1 の回帰ガード）。
# 分境界をまたぐと 2 回目の必要窓が正しく 1 分先になり揺れるため、分の頭から離れて走らせる。
while :; do
  s=$(( $(date +%s) % 60 ))
  [ "$s" -ge 1 ] && [ "$s" -lt 45 ] && break
  sleep 1
done
L="$(case_dir idempotent)"   # lock 未作成＝cold start も併せて踏む
out1="$(run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
end1="$(cat "$L/end" 2>/dev/null || echo '')"
# **秒針を必ず進めてから 2 回目を走らせる**。連続実行だと `date +%s` が同値になり、丸めを
# 外した実装でも cur_end == END_EPOCH で通ってしまう（変異検査でこの穴を踏んだ）。
# 実運用の launchd は StartInterval=300 秒なので秒針は必ず変わる——その条件を再現する。
sleep 1
out2="$(run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if started "$out1" && ! started "$out2" && grep -q '延長不要' <<<"$out2"; then
  ok "同じ post_time なら 2 回目は据え置き（判定が秒針で揺れない・cold start 経由）"
else
  ng "同じ post_time なら 2 回目は据え置き" "out1=$out1 / out2=$out2"
fi
if [ -n "$end1" ] && [ "$((end1 % 60))" -eq 0 ]; then
  ok "記録される end は分境界に丸められている"
else
  ng "記録される end は分境界に丸められている" "end=${end1}"
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
# **`wait` は使えない**: spawn は `$( )` の中で起きるので、この sleep は当シェルの子ではなく
# `wait` は即エラーで返る。SIGTERM 配送前に先へ進むと `kill -0` が成功し、スタブ ps は生きた pid に
# 必ず caffeinate を返すので「稼働中・据え置き」に落ちて flaky に失敗する。死を有界ポーリングで待つ。
dead="$(spawn_fake_caffeinate)"; kill "$dead" 2>/dev/null
for _ in $(seq 1 100); do kill -0 "$dead" 2>/dev/null || break; sleep 0.05; done
if kill -0 "$dead" 2>/dev/null; then
  ng "stale ケースの前提（偽 caffeinate の停止）" "pid ${dead} が 5 秒で死ななかった"
fi
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
# 旧パスも env で注入する。実運用パス（/tmp/paddock-keep-awake.lock.d）を掴むと本物の
# caffeinate を kill しうるので、**skip で逃げず TESTROOT へ逃がす**（skip だと移行ロジックの
# 回帰が無検査になる＝偽陰性）。
L="$(case_dir legacy_path)"
LEGACY="$TESTROOT/case-legacy_path/legacy.lock.d"
mkdir -p "$LEGACY"
caf="$(spawn_fake_caffeinate)"
echo "$caf" > "$LEGACY/pid"
out="$(TEST_LEGACY_DIR="$LEGACY" run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
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
fi

# --- 7b. 据え置き経路でも旧パスの caffeinate を孤児にしない ---
# 旧 lock の削除を early exit より前に置くと、生きた caffeinate の pid 記録だけが消えて
# uninstall からも次サイクルからも止められなくなる。据え置き（＝現行が窓を満たす）でも
# 引き継いだ分は停止し、旧ディレクトリを消し切ることを固定する。
L="$(case_dir legacy_keep)"; mkdir -p "$L"
cur="$(spawn_fake_caffeinate)"
echo "$cur" > "$L/pid"
echo "$(( ($(date +%s) + 86400) / 60 * 60 ))" > "$L/end"   # 現行が窓を満たす
LEGACY2="$TESTROOT/case-legacy_keep/legacy.lock.d"
mkdir -p "$LEGACY2"
old="$(spawn_fake_caffeinate)"
echo "$old" > "$LEGACY2/pid"
out="$(TEST_LEGACY_DIR="$LEGACY2" run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if ! started "$out" && grep -q '延長不要' <<<"$out" && [ ! -d "$LEGACY2" ]; then
  ok "据え置きでも旧パスの caffeinate を停止し旧 lock を消す（孤児を作らない）"
else
  ng "据え置きでも旧パスの caffeinate を始末する" "legacy残=$([ -d "$LEGACY2" ] && echo yes || echo no) out=$out"
fi
if kill -0 "$cur" 2>/dev/null && ! kill -0 "$old" 2>/dev/null; then
  ok "据え置き時に落とすのは旧パス側だけ（現行 caffeinate は生存）"
else
  ng "据え置き時に落とすのは旧パス側だけ" "cur=$(kill -0 "$cur" 2>/dev/null && echo alive || echo dead) old=$(kill -0 "$old" 2>/dev/null && echo alive || echo dead)"
fi

# --- 7c. 早期 exit する経路では旧 lock を消さない（記録を失わない） ---
# 「別プロセスが起動中」で抜けるとき旧ディレクトリまで消すと、生きた caffeinate の記録が
# 失われる。次サイクルが引き継げるよう残すこと。
L="$(case_dir legacy_earlyexit)"; mkdir -p "$L"   # pid 未記入かつ新しい lock＝起動中
LEGACY3="$TESTROOT/case-legacy_earlyexit/legacy.lock.d"
mkdir -p "$LEGACY3"
old3="$(spawn_fake_caffeinate)"
echo "$old3" > "$LEGACY3/pid"
out="$(TEST_LEGACY_DIR="$LEGACY3" run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if grep -q '別プロセスが起動中' <<<"$out" && [ -d "$LEGACY3" ] && kill -0 "$old3" 2>/dev/null; then
  ok "早期 exit する経路では旧 lock も旧 caffeinate も残す（次サイクルが引き継げる）"
else
  ng "早期 exit で旧 lock を消さない" "legacy残=$([ -d "$LEGACY3" ] && echo yes || echo no) out=$out"
fi

# --- 7d. 現行 lock と旧パスの両方が生きている → 両方停止する ---
# 置き換え対象を単一変数で持つと片方を黙って取りこぼし、落とせなかった方が孤児になる。
L="$(case_dir legacy_both)"; mkdir -p "$L"
cur4="$(spawn_fake_caffeinate)"
echo "$cur4" > "$L/pid"
echo "$(( ($(date +%s) + 60) / 60 * 60 ))" > "$L/end"   # 窓が足りない＝延長する
LEGACY4="$TESTROOT/case-legacy_both/legacy.lock.d"
mkdir -p "$LEGACY4"
old4="$(spawn_fake_caffeinate)"
echo "$old4" > "$LEGACY4/pid"
out="$(TEST_LEGACY_DIR="$LEGACY4" run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if started "$out" && ! kill -0 "$cur4" 2>/dev/null && ! kill -0 "$old4" 2>/dev/null; then
  ok "現行 lock と旧パスの両方が生きていれば両方停止する（取りこぼさない）"
else
  ng "現行 lock と旧パスの両方を停止する" "cur=$(kill -0 "$cur4" 2>/dev/null && echo alive || echo dead) old=$(kill -0 "$old4" 2>/dev/null && echo alive || echo dead) out=$out"
fi

# --- 7e. uninstall.sh も旧パスの caffeinate を止める（作成側だけ直すと止められない） ---
# 新コードの tick が 1 度も走らないうちに夜の uninstall を叩く移行経路。
L="$(case_dir uninstall_legacy)"   # 新パスには何も無い
LEGACY5="$TESTROOT/case-uninstall_legacy/legacy.lock.d"
mkdir -p "$LEGACY5"
old5="$(spawn_fake_caffeinate)"
echo "$old5" > "$LEGACY5/pid"
out="$(run_uninstall "$L" "$LEGACY5")"
if grep -q '旧 lock パス' <<<"$out" && ! kill -0 "$old5" 2>/dev/null && [ ! -d "$LEGACY5" ]; then
  ok "uninstall.sh は旧 lock パスの caffeinate も停止して片付ける"
else
  ng "uninstall.sh が旧 lock パスの caffeinate を停止する" "old=$(kill -0 "$old5" 2>/dev/null && echo alive || echo dead) legacy残=$([ -d "$LEGACY5" ] && echo yes || echo no) out=$out"
fi

# --- 8. 既定 lock パスが uid スコープで、作成側と削除側が同じ式を持つ ---
# #643 の要件「作成側と削除側の両方を同時に直す」の機械的な担保。片方だけ変えると
# uninstall が caffeinate を止められなくなる（＝抑止が居座る）。
CREATOR_SH="$REPO_ROOT/scripts/predict-check/keep_awake.sh"
REMOVER_SH="$REPO_ROOT/deployments/launchd/uninstall.sh"
# literal 完全一致で固定すると整形しただけで落ちるので、**両者の定義行を突き合わせる**
# （守りたいのは「同じパスに解決されること」であって字面そのものではない）。
# そのうえで uid スコープであること・env で注入できることを形で確認する。
for var in LOCK_DIR LEGACY_LOCK_DIR; do
  c="$(grep "^${var}=" "$CREATOR_SH" || true)"
  r="$(grep "^${var}=" "$REMOVER_SH" || true)"
  if [ -n "$c" ] && [ "$c" = "$r" ]; then
    ok "作成側と削除側の ${var} 定義が一致する"
  else
    ng "作成側と削除側の ${var} 定義が一致する" "keep_awake.sh=[${c}] uninstall.sh=[${r}]"
  fi
done
lock_def="$(grep '^LOCK_DIR=' "$CREATOR_SH" || true)"
if grep -q 'id -u' <<<"$lock_def" && grep -q 'PADDOCK_KEEP_AWAKE_LOCK_DIR' <<<"$lock_def"; then
  ok "LOCK_DIR は uid スコープかつ env で注入できる"
else
  ng "LOCK_DIR は uid スコープかつ env で注入できる" "def=[${lock_def}]"
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
# **期待ケース数を固定する**。FAIL=0 だけを見ると、条件分岐でケースが丸ごと実行されなかったとき
# （旧実装の skip 分岐がこの形だった）に「全部通った」と読めてしまう＝偽陰性。
EXPECTED=24
if [ "$((pass + fail))" -ne "$EXPECTED" ]; then
  echo "NG  実行ケース数が期待と違う: $((pass + fail)) != ${EXPECTED}（ケースが飛ばされている）"
  exit 1
fi
[ "$fail" -eq 0 ]
