#!/usr/bin/env bash
# prefetch_odds.sh の lock パス（#651）の回帰テスト。
#
# **このスクリプトは開催日にしか本番を踏まない**（launchd の prefetch は開催日の朝に install する
# 運用）。壊れても次の開催まで気づけないので、判断分岐をここで固定する。落とすのは再取得不能な
# 発走直前 snapshot なので、静かに壊れることを許さない。
#
# netkeiba にも DB にも触らない。PATH 差し替えのスタブで
#   `python3` … 対象レース選択（upcoming_races_db.py）には固定の race_id を返し、
#               nk_id 変換（`python3 - <race_id>`）には失敗を返す。
#               **変換失敗にするのは実 fetch へ進ませないため**——開発機には
#               target/release/paddock-fetch-card が実在するので、ここで止めないと本物の
#               netkeiba スクレイプが走る（CI は binary 不在で手前の check に落ちる）。
# を用意し、lock は 4 つの env を全部テスト用ディレクトリへ逃がして実運用の
# /tmp/paddock-prefetch* には一切触らない（稼働中の本物を止めない）。
#
# **PATH は $STUB だけに絞る**。prefetch_odds.sh は `command -v flock` の有無で lock 機構が
# 変わり、本番（macOS）は flock 不在＝mkdir 分岐、ubuntu は flock 分岐に入る。素の PATH で
# 走らせると CI では mkdir 分岐が 1 度も検査されない（＝本番経路が無検査）。必要なコマンドだけを
# symlink し、flock を張らないことで本番と同じ分岐を踏ませる。flock がある環境では flock 分岐の
# ケースも追加で回す。
#
# 使い方: bash scripts/test-prefetch-odds.sh   （全ケース PASS で exit 0）
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1
REPO_ROOT="$PWD"
SCRIPT="$REPO_ROOT/scripts/predict-check/prefetch_odds.sh"
# 全角混じりメッセージを UTF-8 として扱う（#636 の罠を実行時に踏ませる意図）。
# `en_US.UTF-8` は runner で生成されているとは限らず、`C.UTF-8` は glibc / macOS 双方に在る。
export LANG=C.UTF-8 LC_ALL=C.UTF-8
unset PADDOCK_DB_URL WORKDIR WINDOW_MIN
unset PADDOCK_PREFETCH_LOCK PADDOCK_PREFETCH_LOCK_DIR
unset PADDOCK_PREFETCH_LEGACY_LOCK PADDOCK_PREFETCH_LEGACY_LOCK_DIR
# worktree から叩くと GIT_DIR 等が継承されて本物の index を汚す（#645 の実害）。
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE

pass=0
fail=0
skip=0
ok()   { echo "OK  $1"; pass=$((pass + 1)); }
ng()   { echo "NG  $1"; shift; [ $# -gt 0 ] && echo "    $*"; fail=$((fail + 1)); }
note() { echo "--  $1"; skip=$((skip + 1)); }

TESTROOT="$(mktemp -d "${TMPDIR:-/tmp}/paddock-prefetch-test.XXXXXX")"
cleanup() {
  # flock を握らせた背景プロセスを始末する（実機に sleep を残さない）。
  if [ -f "$TESTROOT/spawned" ]; then
    while read -r p; do
      [ -n "$p" ] || continue
      kill "$p" 2>/dev/null
    done < "$TESTROOT/spawned"
  fi
  rm -rf "$TESTROOT"
  return 0
}
trap cleanup EXIT
: > "$TESTROOT/spawned"

# ---- スタブ -----------------------------------------------------------------
STUB="$TESTROOT/bin"
mkdir -p "$STUB"

# prefetch_odds.sh が PATH 経由で使う外部コマンドだけを張る（flock は**わざと張らない**）。
# bash / env はスクリプト本体とスタブの shebang を解決するために要る。
for c in bash env date tee mkdir find rmdir id dirname sleep; do
  p="$(command -v "$c")" || { echo "前提コマンドが無い: $c" >&2; exit 2; }
  # **絶対パスであることを確かめる**。`command -v` は関数・エイリアス・ビルトインでは名前だけを
  # 返すので、そのまま `ln -s` すると自分自身を指す壊れた symlink になり、スタブは exit 127 を
  # 返しながらテストは「判定が偽だった」ように緑～偽 NG を出す（実際に踏んだ）。
  case "$p" in
    /*) ;;
    *)  echo "外部コマンドとして解決できない: ${c}（command -v の結果: ${p}）" >&2; exit 2 ;;
  esac
  ln -s "$p" "$STUB/$c"
done

cat > "$STUB/python3" <<'EOS'
#!/usr/bin/env bash
# prefetch_odds.sh の python3 呼び出しは 2 種類ある。
#   1) upcoming_races_db.py <DATE> --window-min N [--at HH:MM]  … 対象レース選択
#   2) - <race_id>（heredoc をスクリプトとして読む）             … nk_id 変換
# 1 は固定の race_id を返し、2 は失敗させて実 fetch へ進ませない。
if [ "${1-}" = "-" ]; then
  echo "nk_id: テストスタブは変換しない" >&2
  exit 1
fi
printf '%s\n' "${FAKE_RACE_IDS:-2026-3-tokyo-5-6R}"
EOS
chmod +x "$STUB/python3"

# ---- ヘルパ -----------------------------------------------------------------
# run_prefetch <case-name> [args...] : スタブ PATH で prefetch_odds.sh を走らせ、出力を stdout へ、
# 終了コードを**自身の終了コードとして**返す。lock 4 つ・WORKDIR・LOG はケース専用ディレクトリへ逃がす。
# 呼び出し側は `out="$(run_prefetch x)"; rc=$?` で受ける——この関数は $( ) の中＝サブシェルで走るので、
# 変数に置いた終了コードは親へ伝わらない（1 巡目でこれを踏み、rc 判定が全ケース素通りした）。
run_prefetch() {
  local name="$1"; shift
  local d="$TESTROOT/case-$name"
  local out rc
  out="$(PATH="${RUN_PATH:-$STUB}" \
    PADDOCK_PREFETCH_LOCK="$d/new.lock" \
    PADDOCK_PREFETCH_LOCK_DIR="$d/new.lock.d" \
    PADDOCK_PREFETCH_LEGACY_LOCK="$d/legacy.lock" \
    PADDOCK_PREFETCH_LEGACY_LOCK_DIR="$d/legacy.lock.d" \
    PADDOCK_PREFETCH_LOG="$d/prefetch.log" \
    WORKDIR="$d/work" \
    PADDOCK_DB_URL="postgres://p:p@127.0.0.1:1/paddock" \
    FAKE_RACE_IDS="${FAKE_RACE_IDS:-2026-3-tokyo-5-6R}" \
    bash "$SCRIPT" --date 2026-08-22 "$@" 2>&1)"
  rc=$?
  printf '%s' "$out"
  return "$rc"
}
case_dir() { local d="$TESTROOT/case-$1"; mkdir -p "$d"; printf '%s' "$d"; }
# lock を取れたか。取得後に必ず通る 2 経路（binary 不在 / fetch 開始）のどちらかで判定する。
# lock ディレクトリ自体は trap で消えるため、実行後の残骸では判定できない。
acquired() { grep -qE 'release バイナリが見つかりません|prefetch 開始:' <<<"$1"; }
# mtime を時効（30 分）より確実に古くする。-t は BSD / GNU 双方にある POSIX 形式。
age_out() { touch -t 200001010000 "$1"; }
wait_for_file() {
  local f="$1" i=0
  while [ "$i" -lt 100 ]; do
    [ -e "$f" ] && return 0
    i=$((i + 1)); sleep 0.05
  done
  return 1
}

echo "=== 静的検査: lock パスの式 ==="

# --- 1. 既定パスが UID スコープであること ---
# 「片方だけ変えたら落ちる」番人。式を uid 無しへ戻すとここで落ちる。
if grep -q 'PADDOCK_PREFETCH_LOCK:-/tmp/paddock-prefetch-\$(id -u)\.lock}' "$SCRIPT" \
   && grep -q 'PADDOCK_PREFETCH_LOCK_DIR:-/tmp/paddock-prefetch-\$(id -u)\.lock\.d}' "$SCRIPT"; then
  ok "既定 lock パスが UID スコープ（/tmp/paddock-prefetch-\$(id -u).lock{,.d}）"
else
  ng "既定 lock パスが UID スコープ（/tmp/paddock-prefetch-\$(id -u).lock{,.d}）" \
     "LOCK / LOCK_DIR の既定値を確認せよ"
fi

# --- 2. uid 無しの旧パス直書きが旧パス変数以外に残っていない ---
# 移行元として参照する行（LEGACY / 旧 lock）だけを許す。実装のどこかに素の
# /tmp/paddock-prefetch.lock が残っていたら、そこだけ移行から取り残される。
stray="$(grep -n '/tmp/paddock-prefetch\.lock' "$SCRIPT" \
         "$REPO_ROOT/deployments/launchd/com.paddock.prefetch-odds.plist" 2>/dev/null \
         | grep -v 'LEGACY' | grep -v '旧 lock' || true)"
if [ -z "$stray" ]; then
  ok "uid 無しの /tmp/paddock-prefetch.lock 直書きは旧パス参照だけ"
else
  ng "uid 無しの /tmp/paddock-prefetch.lock 直書きは旧パス参照だけ" "$stray"
fi

echo "=== mkdir 分岐（本番 macOS の経路・flock 不在） ==="

# --- 3. 旧 lock が時効内 → 移行前インスタンスが実行中とみなして譲る ---
d="$(case_dir legacy-fresh)"; mkdir -p "$d/legacy.lock.d"
out="$(run_prefetch legacy-fresh)"; rc=$?
if [ "$rc" -eq 0 ] && grep -q '旧 lock パスで別の prefetch 実行中' <<<"$out" \
   && [ -d "$d/legacy.lock.d" ] && [ ! -e "$d/new.lock.d" ]; then
  ok "旧 lock が時効内 → exit 0 で譲り、旧を消さず新 lock も取らない"
else
  ng "旧 lock が時効内 → exit 0 で譲り、旧を消さず新 lock も取らない" "rc=$rc / $out"
fi

# --- 4. 旧 lock が時効切れ → 残骸として片付けて続行 ---
d="$(case_dir legacy-stale)"; mkdir -p "$d/legacy.lock.d"; age_out "$d/legacy.lock.d"
out="$(run_prefetch legacy-stale)"
if grep -q '旧 lock パスの残骸を片付ける' <<<"$out" && [ ! -e "$d/legacy.lock.d" ] && acquired "$out"; then
  ok "旧 lock が時効切れ → 掃除して lock 取得へ進む"
else
  ng "旧 lock が時効切れ → 掃除して lock 取得へ進む" "$out"
fi

# --- 5. 旧 lock が信用できない → 移行チェックを飛ばして続行（止めはしない） ---
d="$(case_dir legacy-evil)"; mkdir -p "$d/real"; ln -s "$d/real" "$d/legacy.lock.d"
out="$(run_prefetch legacy-evil)"
if grep -q '旧 lock パス.*信用できない' <<<"$out" && grep -q '移行チェックをスキップ' <<<"$out" \
   && acquired "$out"; then
  ok "旧 lock が symlink → 警告して移行チェックのみスキップ"
else
  ng "旧 lock が symlink → 警告して移行チェックのみスキップ" "$out"
fi

# --- 6. 新 lock が時効内 → 別の prefetch 実行中としてスキップ ---
d="$(case_dir busy)"; mkdir -p "$d/new.lock.d"
out="$(run_prefetch busy)"; rc=$?
if [ "$rc" -eq 0 ] && grep -q 'mkdir ロック' <<<"$out" && [ -d "$d/new.lock.d" ]; then
  ok "新 lock が時効内 → exit 0 でスキップし lock を奪わない"
else
  ng "新 lock が時効内 → exit 0 でスキップし lock を奪わない" "rc=$rc / $out"
fi

# --- 7. 新 lock が時効切れ → 奪って続行し、終了時に残さない ---
d="$(case_dir stale)"; mkdir -p "$d/new.lock.d"; age_out "$d/new.lock.d"
out="$(run_prefetch stale)"
if grep -q '古いロックを破棄' <<<"$out" && acquired "$out" && [ ! -e "$d/new.lock.d" ]; then
  ok "新 lock が時効切れ → 奪って続行し trap で残さない"
else
  ng "新 lock が時効切れ → 奪って続行し trap で残さない" "$out"
fi

# --- 8. 新 lock が信用できない（symlink） → 大きく警告して非 0 終了 ---
# ここを exit 0 のスキップに倒すと、他ユーザーの居座りで永久に沈黙する（#651 の主眼）。
d="$(case_dir evil)"; mkdir -p "$d/real"; ln -s "$d/real" "$d/new.lock.d"
out="$(run_prefetch evil)"; rc=$?
if [ "$rc" -ne 0 ] && grep -q 'lock パス.*信用できない' <<<"$out" \
   && grep -q 'prefetch を実行できない' <<<"$out"; then
  ok "新 lock が symlink → 警告して非 0 終了（沈黙しない）"
else
  ng "新 lock が symlink → 警告して非 0 終了（沈黙しない）" "rc=$rc / $out"
fi

# --- 9. --dry-run は lock を一切触らない（launchd 実走中でも選択結果を出せる） ---
d="$(case_dir dryrun)"; mkdir -p "$d/new.lock.d" "$d/legacy.lock.d"
out="$(run_prefetch dryrun --dry-run)"; rc=$?
if [ "$rc" -eq 0 ] && grep -q '\[dry-run\] 対象' <<<"$out" \
   && ! grep -q 'スキップ' <<<"$out" && [ -d "$d/legacy.lock.d" ]; then
  ok "--dry-run は lock に阻まれず、旧 lock の移行判定も走らせない"
else
  ng "--dry-run は lock に阻まれず、旧 lock の移行判定も走らせない" "rc=$rc / $out"
fi

# --- 10. 正常な素の状態 → lock を取り、終了後に残さない ---
d="$(case_dir clean)"
out="$(run_prefetch clean)"
if acquired "$out" && [ ! -e "$d/new.lock.d" ] && ! grep -q 'スキップ' <<<"$out"; then
  ok "素の状態 → lock を取得して続行し、終了時に残さない"
else
  ng "素の状態 → lock を取得して続行し、終了時に残さない" "$out"
fi

echo "=== flock 分岐（ubuntu CI の経路） ==="

FLOCK_BIN="$(command -v flock || true)"
if [ -z "$FLOCK_BIN" ]; then
  note "flock 不在のため flock 分岐はスキップ（本番 macOS の実態と一致）"
else
  FLOCK_STUB="$TESTROOT/bin-flock"
  mkdir -p "$FLOCK_STUB"
  for f in "$STUB"/*; do ln -s "$(readlink "$f" || printf '%s' "$f")" "$FLOCK_STUB/$(basename "$f")"; done
  ln -sf "$STUB/python3" "$FLOCK_STUB/python3"
  ln -s "$FLOCK_BIN" "$FLOCK_STUB/flock"

  # 指定ファイルの flock を背景プロセスに握らせる。
  hold_flock() {
    local file="$1" marker="$2"
    # **stdio を必ず /dev/null へ落とす**。背景プロセスが親の stdout パイプを握ったままだと、
    # テスト本体が終わっても読み手（CI のログ収集・`| tail` 等）が sleep の終了まで待たされる
    # ——実測で 2 分ぶん張り付いた。
    ( exec 9>>"$file"; flock -n 9 || exit 1; : > "$marker"; sleep 60 ) >/dev/null 2>&1 &
    echo "$!" >> "$TESTROOT/spawned"
    # cleanup の kill で「Terminated: 15」がテスト出力に混ざるのを止める（CI ログのノイズ）。
    disown
    wait_for_file "$marker"
  }

  # --- 11. 旧 lock ファイルを握られている → 移行前インスタンスに譲る ---
  d="$(case_dir f-legacy-held)"; : > "$d/legacy.lock"
  if hold_flock "$d/legacy.lock" "$d/held"; then
    out="$(RUN_PATH="$FLOCK_STUB" run_prefetch f-legacy-held)"; rc=$?
    if [ "$rc" -eq 0 ] && grep -q '旧 lock パスで別の prefetch 実行中' <<<"$out"; then
      ok "flock: 旧 lock を握られている → exit 0 で譲る"
    else
      ng "flock: 旧 lock を握られている → exit 0 で譲る" "rc=$rc / $out"
    fi
  else
    ng "flock: 旧 lock を握られている → exit 0 で譲る" "背景の flock 取得がタイムアウト"
  fi

  # --- 12. 新 lock ファイルを握られている → スキップ ---
  d="$(case_dir f-held)"; : > "$d/new.lock"
  if hold_flock "$d/new.lock" "$d/held"; then
    out="$(RUN_PATH="$FLOCK_STUB" run_prefetch f-held)"; rc=$?
    if [ "$rc" -eq 0 ] && grep -q '別の prefetch 実行中のためスキップ' <<<"$out"; then
      ok "flock: 新 lock を握られている → exit 0 でスキップ"
    else
      ng "flock: 新 lock を握られている → exit 0 でスキップ" "rc=$rc / $out"
    fi
  else
    ng "flock: 新 lock を握られている → exit 0 でスキップ" "背景の flock 取得がタイムアウト"
  fi

  # --- 13. 新 lock ファイルが symlink → 非 0 終了 ---
  d="$(case_dir f-evil)"; : > "$d/real"; ln -s "$d/real" "$d/new.lock"
  out="$(RUN_PATH="$FLOCK_STUB" run_prefetch f-evil)"; rc=$?
  if [ "$rc" -ne 0 ] && grep -q 'lock パス.*信用できない' <<<"$out"; then
    ok "flock: 新 lock が symlink → 警告して非 0 終了"
  else
    ng "flock: 新 lock が symlink → 警告して非 0 終了" "rc=$rc / $out"
  fi

  # --- 14. 素の状態 → lock を取って続行 ---
  d="$(case_dir f-clean)"
  out="$(RUN_PATH="$FLOCK_STUB" run_prefetch f-clean)"
  if acquired "$out" && ! grep -q 'スキップ' <<<"$out"; then
    ok "flock: 素の状態 → lock を取得して続行"
  else
    ng "flock: 素の状態 → lock を取得して続行" "$out"
  fi
fi

echo
echo "PASS=$pass FAIL=$fail SKIP=$skip"
[ "$fail" -eq 0 ] || exit 1
exit 0
