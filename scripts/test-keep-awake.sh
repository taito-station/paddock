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
# 全角混じりメッセージを UTF-8 として扱う（#636 の罠を実行時に踏ませる意図）。
# `en_US.UTF-8` は runner で生成されているとは限らず、無いと**黙って C にフォールバックして**
# UTF-8 前提が崩れる。`C.UTF-8` は glibc / macOS 双方で確実に存在する。
export LANG=C.UTF-8 LC_ALL=C.UTF-8
unset PADDOCK_DB_URL PADDOCK_KEEP_AWAKE_LOCK_DIR PADDOCK_KEEP_AWAKE_LEGACY_LOCK_DIR WORKDIR
# worktree から叩くと GIT_DIR 等が継承されて本物の index を汚す（#645 の実害）。
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE

pass=0
fail=0
ok()  { echo "OK  $1"; pass=$((pass + 1)); }
ng()  { echo "NG  $1"; shift; [ $# -gt 0 ] && echo "    $*"; fail=$((fail + 1)); }
# caffeinate の**起動を試みたか**。生存までは見ない（起動直後に死ぬケースでも真になる）。
# **スタブが書くログではなく keep_awake.sh 自身の出力で判定する**——スタブは nohup された
# 背景プロセスなので、書き込みが親の終了に間に合わず競合する。
# 親が同期的に出す「起動（pid ...）」行なら取りこぼさない。
tried_start() { grep -q '起動（pid' <<<"$1"; }

# プロセスが死ぬのを有界で待つ。SIGTERM の配送は非同期なので、`kill` 直後に `! kill -0` を
# アサートすると負荷の高い環境で偽 FAIL する。
wait_gone() {
  local p="$1" i=0
  while [ "$i" -lt 100 ]; do
    kill -0 "$p" 2>/dev/null || return 0
    i=$((i + 1)); sleep 0.05
  done
  return 1
}
# スタブが書くログを読むときの待ち合わせ。スタブは nohup された背景プロセスなので、
# 書き込みが親の `$( )` の終了に間に合わない（1 巡目でこの競合を踏んだ）。有界で待つ。
wait_for_line() {
  local file="$1" line="$2" i=0
  while [ "$i" -lt 100 ]; do
    grep -qxF -- "$line" "$file" 2>/dev/null && return 0
    i=$((i + 1)); sleep 0.05
  done
  return 1
}

TESTROOT="$(mktemp -d "${TMPDIR:-/tmp}/paddock-keep-awake-test.XXXXXX")"
cleanup() {
  # 取り残した偽 caffeinate（sleep）を確実に始末する。テストが実機に sleep を残さない。
  # **identity を確かめてから kill する**——記録した pid が既に死んで再利用されていると
  # 無関係なプロセスを落とす（本体側で comm 照合を入れて潰したのと同じ穴）。
  if [ -f "$TESTROOT/spawned" ]; then
    while read -r p; do
      [ -n "$p" ] || continue
      case "$(ps -p "$p" -o comm= 2>/dev/null)" in
        *sleep|*caffeinate) kill "$p" 2>/dev/null ;;
      esac
    done < "$TESTROOT/spawned"
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
# FAKE_CAFFEINATE_DIE=1 なら起動直後に死ぬ（exec 失敗・即死のシミュレート）。
# spawned に載せないので ps スタブも caffeinate と認めない。
if [ -n "${FAKE_CAFFEINATE_DIE-}" ]; then exit 1; fi
printf '%s\n' "$$" >> "${FAKE_SPAWNED_LOG:?}"
exec sleep 300
EOS

cat > "$STUB/ps" <<'EOS'
#!/usr/bin/env bash
# `ps -p PID -o comm=` だけを模す。
# **既定は「caffeinate ではない」**——生きている pid に一律 caffeinate を返すと、comm 照合を
# 省いた実装でも全ケース通ってしまう（偽陰性）。偽 caffeinate として起こした pid は
# FAKE_SPAWNED_LOG に載るので、それを唯一の真とする。
# FAKE_PS_ALIEN_PID は明示的に別プロセス名を返す上書き（PID 再利用の誤判定を潰す分岐用）。
pid=""
while [ $# -gt 0 ]; do
  case "$1" in
    -p) pid="${2-}"; shift 2 ;;
    -o) shift 2 ;;
    *)  shift ;;
  esac
done
[ -n "$pid" ] || exit 0
if [ -n "${FAKE_PS_ALIEN_PID-}" ] && [ "$pid" = "${FAKE_PS_ALIEN_PID}" ]; then
  echo "Dock"; exit 0
fi
# 「caffeinate」を名前に含むが caffeinate ではないプロセス。アンカー無しの部分一致で
# 照合している実装を通さないための餌。
if [ -n "${FAKE_PS_PATHY_PID-}" ] && [ "$pid" = "${FAKE_PS_PATHY_PID}" ]; then
  echo "/tmp/caffeinate-decoy/foo"; exit 0
fi
kill -0 "$pid" 2>/dev/null || exit 0
if grep -qx "$pid" "${FAKE_SPAWNED_LOG:?}" 2>/dev/null; then
  # 実運用の macOS は comm をフルパスで返しうる。アンカー無しの部分一致に頼った実装を
  # 通さないよう、テストでもフルパス形で返す。
  echo "/usr/bin/caffeinate"
else
  echo "some-other-process"
fi
exit 0
EOS

cat > "$STUB/launchctl" <<'EOS'
#!/usr/bin/env bash
# 実 launchd に触らせないための番兵。uninstall.sh の unload はここで空振りする。
printf 'launchctl %s\n' "$*" >> "${FAKE_LAUNCHCTL_LOG:?}"
exit 0
EOS

chmod +x "$STUB/psql" "$STUB/caffeinate" "$STUB/ps" "$STUB/launchctl"

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

# uninstall.sh を同じスタブ PATH で走らせる。
#
# **`HOME` を必ず差し替える**。uninstall.sh は `$HOME/Library/LaunchAgents/com.paddock.*.plist` を
# `launchctl unload` して `rm -f` するので、素で叩くと**実機の稼働中エージェント 3 本が消える**。
# CI（ubuntu）は plist が無いので常に緑になり、この危険は CI では可視化されない。
# `launchctl` もスタブして、万一 plist を拾っても実 launchd に触れないようにする（二重防御）。
run_uninstall() {
  local lockdir="$1" legacydir="$2"
  mkdir -p "$TESTROOT/home/Library/LaunchAgents"
  # **ダミー plist を置く**。置かないと `launchctl unload` の経路自体が実行されず、
  # スタブは呼ばれないまま「二重防御が効いている」と言うことになる（死んだ計装）。
  : > "$TESTROOT/home/Library/LaunchAgents/com.paddock.keep-awake.plist"
  PATH="$STUB:$PATH" \
  HOME="$TESTROOT/home" \
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
export FAKE_LAUNCHCTL_LOG="$TESTROOT/launchctl.args"
: > "$FAKE_CAFFEINATE_LOG"
: > "$FAKE_SPAWNED_LOG"
: > "$FAKE_LAUNCHCTL_LOG"
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
if ! tried_start "$out" && grep -q '延長不要' <<<"$out"; then
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
# 引数ログは全ケース共有の追記ファイルなので、このケースの分だけを見るために切る
# （切らないと、前にケースを 1 つ足しただけで別ケースの行に一致する偽陽性になる）。
: > "$FAKE_CAFFEINATE_LOG"
caf="$(spawn_fake_caffeinate)"
echo "$caf" > "$L/pid"
echo "$(($(date +%s) + 60))" > "$L/end"      # 1 分後まで＝18:30 まで足りない
out="$(run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if tried_start "$out" && grep -q '抑止窓を延長する' <<<"$out"; then
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
# **窓の長さそのものを固定する**。「起動した / しない」しか見ないと、END_MIN / SECS の計算が
# 壊れても全ケース通る（スタブは引数を記録しているのに誰も読んでいない＝死んだ計装だった）。
# --at 10:00（=600 分）/ 最終 post 18:30（=1110 分）+ buffer 10 分 → (1120-600)*60 = 31200 秒。
if grep -q 'caffeinate -i -t 31200s 起動' <<<"$out" && wait_for_line "$FAKE_CAFFEINATE_LOG" '-i -t 31200'; then
  ok "caffeinate に渡す抑止秒数が最終 post + buffer から正しく出ている（-i -t 31200）"
else
  ng "caffeinate の抑止秒数" "記録された引数: $(tr '\n' '/' < "$FAKE_CAFFEINATE_LOG") / out=$out"
fi

# --- 2b. 同じ post_time なら 2 回目は必ず据え置き（判定が実行時刻の秒針で揺れないこと） ---
# END_EPOCH を分境界へ丸めないと、end に実行時刻の秒針が乗る（SECS は分粒度なので
# end = 真の終了時刻 + 秒針）。すると次サイクルの `cur_end >= END_EPOCH` が
# 「前回の秒針 >= 今回の秒針」に退化し、**post_time が変わらなくても延長し続ける**。
# 秒針は必ず進むので、丸めを外すとこのケースは決定的に落ちる（＝R-1 の回帰ガード）。
# **秒針を必ず進めてから 2 回目を走らせる**。連続実行だと `date +%s` が同値になり、丸めを
# 外した実装でも cur_end == END_EPOCH で通ってしまう（変異検査でこの穴を踏んだ）。
# 実運用の launchd は StartInterval=300 秒なので秒針は必ず変わる——その条件を再現する。
#
# 逆に**分をまたぐと 2 回目の必要窓が正しく 1 分先になる**ので延長が正解になり、判定できない。
# 壁時計の待ち合わせで避けると遅い runner で崩れるため、**またいだらやり直す**（決定的）。
idem_attempt=0
while :; do
  idem_attempt=$((idem_attempt + 1))
  if [ "$idem_attempt" -gt 5 ]; then
    echo "ABORT 冪等ケースが 5 回とも分境界をまたいだ（実行が異常に遅い）" >&2
    exit 1
  fi
  L="$(case_dir "idempotent-${idem_attempt}")"   # lock 未作成＝cold start も併せて踏む
  m0=$(( $(date +%s) / 60 ))
  out1="$(run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
  end1="$(cat "$L/end" 2>/dev/null || echo '')"
  sleep 1
  out2="$(run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
  [ "$m0" -eq "$(( $(date +%s) / 60 ))" ] && break
done
if tried_start "$out1" && ! tried_start "$out2" && grep -q '延長不要' <<<"$out2"; then
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
if tried_start "$out" && grep -q '旧形式' <<<"$out"; then
  ok "end 未記入の lock は安全側に倒して張り直す"
else
  ng "end 未記入の lock は張り直す" "out=$out"
fi

# --- 4. pid 未記入かつ新しい lock → 「起動中」で据え置き（既存 self-heal を壊していない） ---
L="$(case_dir starting)"; mkdir -p "$L"
out="$(run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if ! tried_start "$out" && grep -q '別プロセスが起動中' <<<"$out"; then
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
# 前提が崩れたら**即座に落とす**。`ng` で数えると総数がずれ、最後の集計が
# 「ケースが飛ばされている」という実際とは逆の診断を出す。
if kill -0 "$dead" 2>/dev/null; then
  echo "ABORT stale ケースの前提が崩れた: pid ${dead} が 5 秒で死ななかった" >&2
  exit 1
fi
echo "$dead" > "$L/pid"
echo "$(($(date +%s) + 86400))" > "$L/end"   # end は十分先でも pid が死んでいれば取り直す
out="$(run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if tried_start "$out" && grep -q 'stale lock' <<<"$out"; then
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
if tried_start "$out" && grep -q 'stale lock' <<<"$out"; then
  ok "pid が caffeinate でなければ stale 扱い（comm 照合が効いている）"
else
  ng "pid が caffeinate でなければ stale 扱い" "out=$out"
fi

# --- 6g. 新しい caffeinate が起動直後に死んだ → 旧を落とさず残す ---
# `nohup ... &` は exec に失敗しても即座に pid を返す。新の生存を確かめずに旧を kill すると
# 抑止がゼロになり、本 PR が潰そうとしている空白そのものを作る。
L="$(case_dir new_dies)"; mkdir -p "$L"
old6="$(spawn_fake_caffeinate)"
echo "$old6" > "$L/pid"
echo "$(( ($(date +%s) + 60) / 60 * 60 ))" > "$L/end"   # 窓が足りない＝延長しようとする
out="$(FAKE_CAFFEINATE_DIE=1 run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if grep -q '起動直後に居ない' <<<"$out" && kill -0 "$old6" 2>/dev/null; then
  ok "新しい caffeinate が起動直後に死んだら旧を落とさず残す（抑止を切らさない）"
else
  ng "新が死んだら旧を残す" "old=$(kill -0 "$old6" 2>/dev/null && echo alive || echo dead) out=$out"
fi

# --- 6e. comm が「caffeinate を含むだけ」のパス → stale 扱い（部分一致で通さない） ---
# macOS の comm はフルパスで返りうるので末尾要素でアンカーする必要がある。
# 部分一致だと /tmp/caffeinate-decoy/foo のような他人のプロセスまで「稼働中」と誤認し、
# 延長経路に入って kill してしまう。
L="$(case_dir pathy_comm)"; mkdir -p "$L"
pathy="$(spawn_fake_caffeinate)"
echo "$pathy" > "$L/pid"
echo "$(( ($(date +%s) + 86400) / 60 * 60 ))" > "$L/end"
out="$(FAKE_PS_PATHY_PID="$pathy" run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if grep -q 'stale lock' <<<"$out" && kill -0 "$pathy" 2>/dev/null; then
  ok "comm が caffeinate を含むだけのパスなら stale 扱い（末尾アンカーが効いている）"
else
  ng "comm の末尾アンカー" "alive=$(kill -0 "$pathy" 2>/dev/null && echo yes || echo no) out=$out"
fi

# --- 6f. pid が数値でない → 記録なし扱い（kill に渡さない） ---
# **`-1` を使うのが要点**。`kill -1` は「プロセスグループ全体へ signal」を意味するので、
# 先回りで仕込まれると自分のプロセスを一掃されうる。数値検証を外すと `pid=-1` として
# stale 経路（＝ログに pid が出る＝kill 対象として扱われた）へ落ちるので、そこで見分ける。
L="$(case_dir corrupt_pid)"; mkdir -p "$L"
printf -- '-1\n' > "$L/pid"
echo "$(( ($(date +%s) + 86400) / 60 * 60 ))" > "$L/end"
out="$(run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if grep -q '別プロセスが起動中' <<<"$out" && ! grep -q 'pid=-1' <<<"$out"; then
  ok "pid が数値でなければ記録なし扱いにする（kill に -1 等を渡さない）"
else
  ng "pid が数値でなければ記録なし扱い" "out=$out"
fi

# --- 6b. end が非数値（破損）→ 安全側に倒して張り直す ---
# `[ x -ge y ]` に非数値を渡すと exit 2 になるので、比較の前に弾く必要がある。
L="$(case_dir corrupt_end)"; mkdir -p "$L"
caf="$(spawn_fake_caffeinate)"
echo "$caf" > "$L/pid"
printf 'not-a-number\n' > "$L/end"
out="$(run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if tried_start "$out" && grep -q '旧形式か破損' <<<"$out"; then
  ok "end が非数値の lock は安全側に倒して張り直す"
else
  ng "end が非数値の lock は張り直す" "out=$out"
fi

# --- 6c. lock が symlink → 中身を信用せず大きく警告して抜ける ---
# /tmp は誰でも名前を作れる。先回りで symlink を張られると `-O` はリンク先を見て真になるため、
# 所有者だけを見る実装では素通りする。中身（pid・end）を読む前に弾くこと。
mkdir -p "$TESTROOT/decoy"
L="$TESTROOT/case-symlink-lock.d"
mkdir -p "$(dirname "$L")"
ln -s "$TESTROOT/decoy" "$L"
out="$(run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if ! tried_start "$out" && grep -q '信用できない' <<<"$out"; then
  ok "lock が symlink なら中身を読まず警告して抜ける（無言停止させない）"
else
  ng "lock が symlink なら警告して抜ける" "out=$out"
fi
rm -f "$L"

# --- 6d. --help がコード行を漏らさない ---
# 行番号固定の `sed` はヘッダ長とズレるとコードを吐く（#585 以前から 10 行漏れていた）。
help_out="$(PATH="$STUB:$PATH" bash "$REPO_ROOT/scripts/predict-check/keep_awake.sh" --help 2>&1)"
if grep -q '環境変数:' <<<"$help_out" \
   && ! grep -q '^set -' <<<"$help_out" \
   && ! grep -q 'help-end' <<<"$help_out" \
   && ! grep -q '^DATE=' <<<"$help_out"; then
  ok "--help はヘッダだけを出しコード行を漏らさない"
else
  ng "--help がコード行を漏らさない" "out=$help_out"
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
if grep -q '旧 lock パスの caffeinate を引き継ぐ' <<<"$out" && tried_start "$out"; then
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

# --- 7a2. 旧パスが symlink → 中身を読まず警告してスキップ（移行を諦める） ---
# 旧パスも `/tmp` 直下なので先回りされうる。pid の中身がそのまま kill の引数になるため、
# 信用できないディレクトリは読まない。
L="$(case_dir legacy_symlink)"
mkdir -p "$TESTROOT/decoy3"
LEGACY_SL="$TESTROOT/case-legacy_symlink/legacy.lock.d"
ln -s "$TESTROOT/decoy3" "$LEGACY_SL"
out="$(TEST_LEGACY_DIR="$LEGACY_SL" run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if grep -q '旧 lock パス.*信用できない' <<<"$out" && [ -L "$LEGACY_SL" ]; then
  ok "旧 lock パスが symlink なら読まず消さず警告する"
else
  ng "旧 lock パスが symlink なら触らない" "残存=$([ -L "$LEGACY_SL" ] && echo yes || echo no) out=$out"
fi
rm -f "$LEGACY_SL"

# --- 7a3. 旧パスに死んだ pid → 残骸として掃除する ---
L="$(case_dir legacy_stale)"
LEGACY_ST="$TESTROOT/case-legacy_stale/legacy.lock.d"
mkdir -p "$LEGACY_ST"
deadl="$(spawn_fake_caffeinate)"; kill "$deadl" 2>/dev/null
wait_gone "$deadl" || { echo "ABORT legacy stale の前提が崩れた" >&2; exit 1; }
echo "$deadl" > "$LEGACY_ST/pid"
out="$(TEST_LEGACY_DIR="$LEGACY_ST" run_keep_awake "$L" --date 2026-08-22 --at 10:00)"
if grep -q '生存せず' <<<"$out" && [ ! -d "$LEGACY_ST" ]; then
  ok "旧 lock パスの死んだ pid は残骸として掃除する"
else
  ng "旧 lock パスの残骸を掃除する" "残存=$([ -d "$LEGACY_ST" ] && echo yes || echo no) out=$out"
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
if ! tried_start "$out" && grep -q '延長不要' <<<"$out" && [ ! -d "$LEGACY2" ]; then
  ok "据え置きでも旧パスの caffeinate を停止し旧 lock を消す（孤児を作らない）"
else
  ng "据え置きでも旧パスの caffeinate を始末する" "legacy残=$([ -d "$LEGACY2" ] && echo yes || echo no) out=$out"
fi
if kill -0 "$cur" 2>/dev/null && wait_gone "$old"; then
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
if tried_start "$out" && wait_gone "$cur4" && wait_gone "$old4"; then
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
if grep -q '旧 lock パス' <<<"$out" && wait_gone "$old5" && [ ! -d "$LEGACY5" ]; then
  ok "uninstall.sh は旧 lock パスの caffeinate も停止して片付ける"
else
  ng "uninstall.sh が旧 lock パスの caffeinate を停止する" "old=$(kill -0 "$old5" 2>/dev/null && echo alive || echo dead) legacy残=$([ -d "$LEGACY5" ] && echo yes || echo no) out=$out"
fi
# **uninstall.sh は $HOME の plist を消しに行く**。テストが実機の稼働中エージェントを外さないよう、
# HOME がサンドボックスへ差し替わっていることを実際の出力で固定する（CI(ubuntu) は plist が
# 無いので素通りし、この危険は CI では可視化されない）。
# ダミー plist を置いてあるので「除去しました」が出る＝**サンドボックス内の plist を実際に処理した**
# ことまで言える（「未インストール」だと差し替えの証明にならない）。
if grep -q "除去しました: ${TESTROOT}/home/Library/LaunchAgents/com.paddock.keep-awake.plist" <<<"$out" \
   && [ ! -e "$TESTROOT/home/Library/LaunchAgents/com.paddock.keep-awake.plist" ]; then
  ok "uninstall.sh の HOME はテスト用サンドボックスに閉じている（実機の plist を触らない）"
else
  ng "uninstall.sh の HOME がサンドボックスに閉じている" "out=$out"
fi
# launchctl スタブが実際に呼ばれたこと＝実 launchd へ行っていないことを固定する。
if grep -q "launchctl unload ${TESTROOT}/home/Library/LaunchAgents/com.paddock.keep-awake.plist" "$FAKE_LAUNCHCTL_LOG"; then
  ok "launchctl はスタブに向いている（実 launchd を触らない）"
else
  ng "launchctl がスタブに向いている" "log=$(cat "$FAKE_LAUNCHCTL_LOG")"
fi

# --- 7f. uninstall.sh の主経路（新パス側）も停止して片付ける ---
# 7e は旧パスだけを踏むので、本来の経路が無検査だった。
L="$(case_dir uninstall_main)"; mkdir -p "$L"
cur7="$(spawn_fake_caffeinate)"
echo "$cur7" > "$L/pid"
out="$(run_uninstall "$L" "$TESTROOT/absent-legacy.lock.d")"
if grep -q "caffeinate を停止しました（pid ${cur7}）" <<<"$out" \
   && wait_gone "$cur7" && [ ! -d "$L" ]; then
  ok "uninstall.sh は新パスの caffeinate を停止して lock を片付ける"
else
  ng "uninstall.sh が新パスの caffeinate を停止する" "alive=$(kill -0 "$cur7" 2>/dev/null && echo yes || echo no) lock残=$([ -d "$L" ] && echo yes || echo no) out=$out"
fi

# --- 7g. uninstall.sh は symlink の lock を触らない ---
L2="$TESTROOT/case-uninstall-symlink.d"
mkdir -p "$TESTROOT/decoy2"
ln -s "$TESTROOT/decoy2" "$L2"
out="$(run_uninstall "$L2" "$TESTROOT/absent-legacy.lock.d")"
if grep -q '信用できない' <<<"$out" && [ -L "$L2" ]; then
  ok "uninstall.sh は symlink の lock を読まず消さない"
else
  ng "uninstall.sh は symlink の lock を触らない" "残存=$([ -L "$L2" ] && echo yes || echo no) out=$out"
fi
rm -f "$L2"

# --- 7h. uninstall.sh は symlink の旧 lock も触らない ---
# 7g は新パスだけを踏むので、旧パス側の信用検査が無検査だった（変異検査で発覚）。
L="$(case_dir uninstall_legacy_symlink)"
LEG_SL2="$TESTROOT/case-uninstall_legacy_symlink/legacy.lock.d"
mkdir -p "$TESTROOT/decoy4"
ln -s "$TESTROOT/decoy4" "$LEG_SL2"
out="$(run_uninstall "$L" "$LEG_SL2")"
if grep -q '旧 lock パス.*信用できない' <<<"$out" && [ -L "$LEG_SL2" ]; then
  ok "uninstall.sh は symlink の旧 lock を読まず消さない"
else
  ng "uninstall.sh は symlink の旧 lock を触らない" "残存=$([ -L "$LEG_SL2" ] && echo yes || echo no) out=$out"
fi
rm -f "$LEG_SL2"

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
EXPECTED=38
if [ "$((pass + fail))" -ne "$EXPECTED" ]; then
  echo "NG  実行ケース数が期待と違う: $((pass + fail)) != ${EXPECTED}（ケースが飛ばされている）"
  exit 1
fi
[ "$fail" -eq 0 ]
