#!/usr/bin/env bash
# 締切前 live オッズの自動 prefetch — 発走 N 分以内のレースの最新オッズを取得し、
# race_odds_snapshots（#232）に締切前 live スナップショットを蓄積する（#237）。
#
# refresh_ev.sh（EV 算出まで行う当日監視ツール）とは別物で、本スクリプトは odds 取得だけに
# 特化する。レース選択は #235 の DB post_time（race_cards.post_time）で行い、netkeiba を
# 都度スクレイプしない。launchd 等から数分間隔で起動される前提（deployments/launchd/）。
#
# 使い方:
#   scripts/predict-check/prefetch_odds.sh [--date YYYY-MM-DD] [--window-min N] [--at HH:MM] [--dry-run]
#   既定 DATE=今日(JST), WINDOW_MIN=30。--dry-run は対象レースの表示のみで fetch しない。
#
# 環境変数:
#   PADDOCK_DB_URL  Postgres 接続 URL（既定: postgres://paddock:paddock@127.0.0.1:5432/paddock）
#                   host は 127.0.0.1 を使う（#212, localhost の ::1 先解決で別 postgres 事故回避）。
#   WORKDIR         scratch 作業ディレクトリ（既定: $TMPDIR/paddock-prefetch。lock は WORKDIR とは
#                   無関係な UID スコープの固定パス。理由は下の acquire_lock 前のコメント）
#   PADDOCK_PREFETCH_LOG  本体ログの出力先ファイル（既定: ~/Library/Logs/paddock-prefetch.log。
#                   /tmp は再起動・periodic clean で消えるため、取りこぼし調査に残る永続パスへ出す #493）
#   PADDOCK_PREFETCH_LOCK / PADDOCK_PREFETCH_LOCK_DIR
#                   多重起動防止 lock の置き場所（既定 /tmp/paddock-prefetch-$(id -u).lock{,.d}・#651）。
#                   flock がある環境は前者（ファイル）、無い環境は後者（ディレクトリ）を使う。
#   PADDOCK_PREFETCH_LEGACY_LOCK / PADDOCK_PREFETCH_LEGACY_LOCK_DIR
#                   移行元の旧 lock（既定 /tmp/paddock-prefetch.lock{,.d}。uid 無しの固定パス）。
#                   テストが実運用の lock を掴んで本物の prefetch を止めないよう env で注入可能にする。
#   WINDOW_MIN      発走まで何分以内を対象にするか（既定 30。引数 --window-min が優先）
#
# 終了コード: 全レース成功 or 対象0件で 0。fetch/変換に 1 件でも失敗したら非 0（#493）。
#   発走直前オッズ snapshot は再取得不能資産のため、失敗を exit 0 に握り潰さず launchd 側へ伝える。
#
# 前提: その日の出馬表（post_time 入り）は朝の paddock-fetch-card 運用で投入済みであること。
# 未投入なら対象 0 件で no-op（正常終了）。
set -euo pipefail

DATE=""
WINDOW_MIN="${WINDOW_MIN:-30}"
AT=""
DRY_RUN=0
while [ $# -gt 0 ]; do
  case "$1" in
    --date) DATE="${2:?--date には YYYY-MM-DD}"; shift 2 ;;
    --window-min) WINDOW_MIN="${2:?--window-min には分}"; shift 2 ;;
    --at) AT="${2:?--at には HH:MM}"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "不明な引数: $1" >&2; exit 2 ;;
  esac
done

# 既定日付は JST の今日（launchd/cron の TZ に依存しないよう明示）。
DATE="${DATE:-$(TZ=Asia/Tokyo date +%Y-%m-%d)}"
[[ "$DATE" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || { echo "DATE は YYYY-MM-DD: $DATE" >&2; exit 2; }
[[ "$WINDOW_MIN" =~ ^[0-9]+$ ]] || { echo "WINDOW_MIN は整数（分）: $WINDOW_MIN" >&2; exit 2; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DB_URL="${PADDOCK_DB_URL:-postgres://paddock:paddock@127.0.0.1:5432/paddock}"
WORKDIR="${WORKDIR:-${TMPDIR:-/tmp}/paddock-prefetch}"
mkdir -p "$WORKDIR"
# 本体ログは永続パス（~/Library/Logs）へ。/tmp は再起動・periodic clean で消え、取りこぼしの
# 事後調査ができなくなる（#493）。PADDOCK_PREFETCH_LOG で上書き可。ディレクトリは作成する。
LOG="${PADDOCK_PREFETCH_LOG:-$HOME/Library/Logs/paddock-prefetch.log}"
mkdir -p "$(dirname "$LOG")"

# tee 失敗（~/Library/Logs 書込不可等）を握る。set -e 下では tee の非0が log 呼び出し行で
# スクリプトを中断し、全成功パスでも exit≠0＝launchd err に誤警報が乗る（#493 レビュー指摘）。
# ログ出力の副作用失敗で本体（fetch 成否）の戻り値を汚さない。
log() { echo "[$(date '+%Y-%m-%dT%H:%M:%S%z')] $*" | tee -a "$LOG" || true; }

# 多重起動防止。launchd の StartInterval と前回実行（ハング含む）が重なっても二重 fetch しない。
# 素の macOS に flock は同梱されないため、flock 不在時は mkdir の原子性で排他するフォールバックを
# 必ず効かせる（cron 代替経路でもノーガードにしない）。本番ホスト（macOS）に flock は無いので
# **実運用で踏むのは mkdir 分岐**。ubuntu の CI は flock 分岐に入るため、両方に同じ規律を持たせる。
#
# lock パスは **WORKDIR に依存しない UID スコープの固定パス**（#651。keep_awake.sh と同じ式）。
# - WORKDIR 配下に置けない: launchd は WORKDIR=/tmp/paddock-prefetch、手動実行は $TMPDIR 配下と
#   WORKDIR が異なるため、両者が別ロックを見て二重 fetch しうる。
# - $TMPDIR も使えない: launchd は TMPDIR を設定せず ${TMPDIR:-/tmp} が /tmp に落ちるので、端末
#   （/var/folders/.../T/）と別の lock を見て互いを見失う（#643 で実測）。
# - uid を挟むのは同一ホストの別ユーザーとの**事故**衝突を避けるため。id -u は launchd 経由でも
#   端末でも同じ値に解決される。**悪意ある先回りは防げない**——/tmp は world-writable で、
#   sticky bit が禁じるのは他人のエントリの削除・改名だけであり、新しい名前の作成は誰でもできる。
#   そこで lock を信用してよいか（symlink でない・自分の所有）を検査し、駄目なら大きく警告して
#   **非 0 で終わる**（#651）。ここを exit 0 のスキップに倒すと、他ユーザーの居座りで下の 30 分
#   stale 回収も rmdir 失敗で効かないまま、外形正常のまま prefetch が永久に沈黙する——落とすのは
#   再取得不能な発走直前 snapshot なので、失敗は握り潰さず launchd の err ログへ伝える（#493）。
#
# 排他が要るのは実 fetch だけなので、取得は dry-run 早期 return の後（fetch 直前）で行う。
# こうすると read-only な --dry-run は launchd 実走中でもロックに阻まれず常に選択結果を表示できる。
LOCK="${PADDOCK_PREFETCH_LOCK:-/tmp/paddock-prefetch-$(id -u).lock}"
LOCK_DIR="${PADDOCK_PREFETCH_LOCK_DIR:-/tmp/paddock-prefetch-$(id -u).lock.d}"
# 旧パス（uid 無し）。移行期に見て、旧 lock を掴んだまま走っている実行中インスタンスに譲る。
# 取りこぼすと新旧が互いを見失って二重 fetch になり、netkeiba への取得回数が倍になる（ADR 0068）。
# テストが実運用の旧 lock を掴んで本物の prefetch を止めないよう、env で注入可能にする。
LEGACY_LOCK="${PADDOCK_PREFETCH_LEGACY_LOCK:-/tmp/paddock-prefetch.lock}"
LEGACY_LOCK_DIR="${PADDOCK_PREFETCH_LEGACY_LOCK_DIR:-/tmp/paddock-prefetch.lock.d}"
# lock を「まだ実行中」とみなす時効（分）。StartInterval(5 分) より十分長い 30 分。
LOCK_STALE_MIN=30

# lock を信用してよいか。/tmp は誰でも名前を作れるので **symlink / 他ユーザー所有は敵対的とみなす**。
# [ -L ] はリンクを辿らないので先に見る（-d / -O は辿るため、自分所有の実体を指す symlink を
# 置かれると単独では素通りする）。ファイル版は書込可否まで見る（flock 前の exec が
# set -e 下でシェルごと落ちるのを避けるため、開く前に弾く）。
lock_dir_is_trustworthy()  { [ ! -L "$1" ] && [ -d "$1" ] && [ -O "$1" ]; }
lock_file_is_trustworthy() { [ ! -L "$1" ] && [ -f "$1" ] && [ -O "$1" ] && [ -w "$1" ]; }
# mtime が時効より古いか＝前回が異常終了した残骸とみなせるか。prefetch の lock は pid を記録しない
# （mkdir + trap rmdir の短命ロック）ので、生存判定の材料は mtime しかない。
lock_is_stale() { [ -n "$(find "$1" -prune -mmin +"$LOCK_STALE_MIN" 2>/dev/null)" ]; }
# 敵対 lock は沈黙させない。exit 0 に倒すと「取れていない」ことに次の開催まで気づけない。
abort_untrustworthy_lock() {
  log "⚠ lock パス $1 が信用できない（symlink / 他ユーザー所有 / 種別違い）。prefetch を実行できない——放置すると発走直前 snapshot を取り続けられない"
  exit 1
}

acquire_lock() {
  if command -v flock >/dev/null 2>&1; then
    # 移行期: 旧ファイルを掴んでいる実行中インスタンスが居れば譲る。fd 8 は本実行の間ずっと
    # 開いたままにする（閉じると旧 lock が解放され、移行前インスタンスが割り込める）。
    # 旧ファイルは**存在するときだけ**見る（作らない。作ると移行が終わらない）。
    if [ "$LOCK" != "$LEGACY_LOCK" ] && { [ -e "$LEGACY_LOCK" ] || [ -L "$LEGACY_LOCK" ]; }; then
      if ! lock_file_is_trustworthy "$LEGACY_LOCK"; then
        log "⚠ 旧 lock パス ${LEGACY_LOCK} が信用できない（ファイルでない / symlink / 他ユーザー所有 / 書込不可）。移行チェックをスキップする"
      else
        exec 8>>"$LEGACY_LOCK"
        flock -n 8 || { log "旧 lock パスで別の prefetch 実行中のためスキップ（移行前のインスタンス）"; exit 0; }
      fi
    fi
    if { [ -e "$LOCK" ] || [ -L "$LOCK" ]; } && ! lock_file_is_trustworthy "$LOCK"; then
      abort_untrustworthy_lock "$LOCK"
    fi
    exec 9>"$LOCK"
    flock -n 9 || { log "別の prefetch 実行中のためスキップ"; exit 0; }
  else
    # 移行期: 旧ディレクトリが時効内に触られていれば「移行前インスタンスが実行中」とみなして譲る。
    # 時効を過ぎていれば残骸として片付ける（旧パスの回収はこの 1 経路だけ・#651）。
    if [ "$LOCK_DIR" != "$LEGACY_LOCK_DIR" ] && { [ -e "$LEGACY_LOCK_DIR" ] || [ -L "$LEGACY_LOCK_DIR" ]; }; then
      if ! lock_dir_is_trustworthy "$LEGACY_LOCK_DIR"; then
        log "⚠ 旧 lock パス ${LEGACY_LOCK_DIR} が信用できない（ディレクトリでない / symlink / 他ユーザー所有）。移行チェックをスキップする"
      elif ! lock_is_stale "$LEGACY_LOCK_DIR"; then
        log "旧 lock パスで別の prefetch 実行中のためスキップ（移行前のインスタンス）: ${LEGACY_LOCK_DIR}"
        exit 0
      else
        log "旧 lock パスの残骸を片付ける（${LOCK_STALE_MIN} 分以上更新なし）: ${LEGACY_LOCK_DIR}"
        rmdir "$LEGACY_LOCK_DIR" 2>/dev/null || true
      fi
    fi
    # **奪う前に lock 自体を検査する**。所有者検査を mkdir 失敗時だけに置くと、他ユーザーの
    # 居座りが「別の prefetch 実行中」という無害な行に化けて沈黙する。
    if { [ -e "$LOCK_DIR" ] || [ -L "$LOCK_DIR" ]; } && ! lock_dir_is_trustworthy "$LOCK_DIR"; then
      abort_untrustworthy_lock "$LOCK_DIR"
    fi
    # 異常終了でロックが残ると永久ブロックするため、一定時間より古いロックは奪う（前回が
    # ハング/強制終了した残骸とみなす）。
    if [ -d "$LOCK_DIR" ] && lock_is_stale "$LOCK_DIR"; then
      log "古いロックを破棄（前回が異常終了した可能性）: ${LOCK_DIR}"
      rmdir "$LOCK_DIR" 2>/dev/null || true
    fi
    if ! mkdir -m 700 "$LOCK_DIR" 2>/dev/null; then
      log "別の prefetch 実行中のためスキップ（mkdir ロック）"
      exit 0
    fi
    trap 'rmdir "$LOCK_DIR" 2>/dev/null || true' EXIT
  fi
}

# paddock race_id（例 2026-3-tokyo-5-6R）→ netkeiba 12 桁。正本は
# src/use-case/src/netkeiba_race_id.rs（CLI 露出が無いため refresh_ev.sh と同じ変換を持つ）。
nk_id() {
  python3 - "$1" <<'PY'
import sys
pid = sys.argv[1]
parts = pid.split("-")  # {年}-{回}-{場slug}-{日}-{R}R
if len(parts) != 5:
    sys.exit(f"nk_id: 想定外の race_id 形式: {pid}")
year, kai, ven, day, rr = parts
vc = {"sapporo": "01", "hakodate": "02", "fukushima": "03", "niigata": "04", "tokyo": "05",
      "nakayama": "06", "chukyo": "07", "kyoto": "08", "hanshin": "09", "kokura": "10"}.get(ven)
if vc is None:
    sys.exit(f"nk_id: 未知の場 slug: {ven}（pid={pid}）")
print(f"{year}{vc}{int(kai):02d}{int(day):02d}{int(rr.rstrip('R')):02d}")
PY
}

# 対象 paddock race_id を DB post_time で選択（#235）。--at はテスト/検証用に現在時刻を上書き。
# command substitution で受けて選択の成否を明示判定する。process substitution（< <(...)）だと
# psql 接続失敗（DB ダウン）でも非0終了が伝播せず「対象0件」と区別不能になり、無人 prefetch が
# 黙って機能停止してもログ上は正常に見えてしまう（Reviewer 指摘）。
SELECT_ARGS=(--window-min "$WINDOW_MIN")
[ -n "$AT" ] && SELECT_ARGS+=(--at "$AT")
if ! SELECTED="$(PADDOCK_DB_URL="$DB_URL" PYTHONPATH="$SCRIPT_DIR" \
      python3 "$SCRIPT_DIR/upcoming_races_db.py" "$DATE" "${SELECT_ARGS[@]}")"; then
  # 失敗要因は DB 接続不可・クエリ失敗のほか、暦上不正な日付（python 側 valid_date が弾く）も
  # ありうるため、原因を断定しない中立な文言にする（「対象0件」とは区別して必ず中断する）。
  log "レース選択コマンドに失敗（DB 接続不可・日付不正・クエリ失敗等）。中断する。"
  exit 1
fi
PIDS=()
while IFS= read -r line; do
  [ -n "$line" ] && PIDS+=("$line")
done <<< "$SELECTED"

if [ "${#PIDS[@]}" -eq 0 ]; then
  log "対象レースなし: $DATE 発走 ${WINDOW_MIN} 分以内の未発走は無し（開催外/朝/全レース終了）"
  exit 0
fi

if [ "$DRY_RUN" -eq 1 ]; then
  log "[dry-run] 対象 ${#PIDS[@]} レース: ${PIDS[*]}"
  exit 0
fi

# ここから実 fetch。多重起動防止のロックを取得（read-only な選択・dry-run は阻まない）。
acquire_lock

# release バイナリ確認（debug ビルドでのライブ運用を防ぐ, refresh_ev.sh と同方針 #211）。
# 実フェッチ時のみ必要なので dry-run の後に置く。
FETCH_BIN="$REPO_ROOT/target/release/paddock-fetch-card"
if [[ ! -x "$FETCH_BIN" ]]; then
  log "release バイナリが見つかりません: $FETCH_BIN"
  log "先に: cd $REPO_ROOT && cargo build --release --bin paddock-fetch-card"
  exit 1
fi

log "prefetch 開始: $DATE 発走 ${WINDOW_MIN} 分以内 ${#PIDS[@]} レース"
FAILED=()
for pid in "${PIDS[@]}"; do
  # race_id 変換失敗（未知 slug 等の異常データ）は 1 件スキップに留め、残りの締切前 prefetch を
  # 巻き添えで止めない（set -e 下の代入失敗で全体中断するのを防ぐ）。
  if ! nk="$(nk_id "$pid")"; then
    log "  SKIP $pid (race_id 変換失敗)"; FAILED+=("$pid"); continue
  fi
  # --force で再取得（既存 race_odds を最新で上書き＋snapshots へ追記）、--skip-history で近走は省く。
  if "$FETCH_BIN" "$nk" --force --skip-history --interval 800 >> "$LOG" 2>&1; then
    log "  ok   $pid ($nk)"
  else
    log "  FAIL $pid ($nk)"; FAILED+=("$pid")
  fi
  sleep 1  # netkeiba への pacing（feedback_jra_fetch_pacing）。fetch-card 内 --interval とは別。
done

if [ "${#FAILED[@]}" -gt 0 ]; then
  # 発走直前 snapshot は再取得不能。失敗を exit 0 に握り潰すと launchd/監視は外形正常のまま
  # 資産を丸ごと失う（stale バイナリ・netkeiba 変化で毎サイクル失敗しても気付けない #493）。
  # ログ追記に加えて非 0 終了で失敗を上位（launchd の err ログ・cron の $?）へ伝える。
  log "prefetch 完了（${#FAILED[@]} 件失敗: ${FAILED[*]}）"
  exit 1
fi
log "prefetch 完了（全 ${#PIDS[@]} レース成功）"
# 正常系（全成功）は明示 exit 0 で締める（keep_awake.sh 流儀。tee 失敗は log() 側で握る二重防御）。
exit 0
