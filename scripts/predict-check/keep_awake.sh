#!/usr/bin/env bash
# 開催日の発走ウィンドウ中、Mac のアイドルスリープを抑止して締切前 prefetch（#237）の
# launchd タイマーを確実に発火させる（#264）。
#
# launchd の StartInterval はスリープ中に発火しないため、無人・離席で画面が寝ると prefetch が
# 取りこぼす（発走直前 snapshot が欠落＝過去オッズ再取得不能）。本スクリプトは当日の最終 post_time
# まで `caffeinate -i` でアイドルスリープを抑止し、prefetch の 5 分タイマーを回し続ける。
#
# **限界（best-effort）**: caffeinate はアイドルスリープを止めるだけで、
#   - クラムシェル（蓋閉じ）スリープや `pmset` のスケジュールスリープは止められない（要 sudo/pmset）
#   - 既にスリープ中の Mac を起こすことはできない（朝にこのジョブが発火する時点で起きている必要がある）
# 完全な堅牢化は常時稼働ホスト（RasPi/小型 VM 等）への prefetch 移設（deployments/launchd/README）。
#
# 使い方:
#   scripts/predict-check/keep_awake.sh [--date YYYY-MM-DD] [--buffer-min N] [--at HH:MM] [--dry-run]
#   既定 DATE=今日(JST), BUFFER_MIN=10。--at は現在時刻の上書き（検証用）、--dry-run は計算のみ。
#
# **抑止窓は毎サイクル追従する（#585）**: 既に caffeinate が稼働していても、DB の最新 post_time から
# 算出した終了時刻が現行の抑止終了時刻より後なら張り直す。朝の install 時点でカードが途中までしか
# 入っていないと窓が短いまま固定され、caffeinate の自然終了から次の launchd tick まで最大
# StartInterval 分（5 分）の抑止空白が空く——2026-08-08 に実発生した。
#
# 環境変数:
#   PADDOCK_DB_URL              Postgres 接続 URL（既定 postgres://paddock:paddock@127.0.0.1:5432/paddock）
#   WORKDIR                     ログ出力先（既定 $TMPDIR/paddock-keep-awake）
#   PADDOCK_KEEP_AWAKE_LOCK_DIR        lock の置き場所（既定 /tmp/paddock-keep-awake-$(id -u).lock.d）。テスト注入用
#   PADDOCK_KEEP_AWAKE_LEGACY_LOCK_DIR 移行元の旧 lock（既定 /tmp/paddock-keep-awake.lock.d）。テスト注入用
#                                      ——これを注入可能にしておかないと、テストが実運用の旧 lock を
#                                      掴んで本物の caffeinate を殺しうる
set -euo pipefail

DATE=""
BUFFER_MIN="${BUFFER_MIN:-10}"
AT=""
DRY_RUN=0
while [ $# -gt 0 ]; do
  case "$1" in
    --date) DATE="${2:?--date には YYYY-MM-DD}"; shift 2 ;;
    --buffer-min) BUFFER_MIN="${2:?--buffer-min には分}"; shift 2 ;;
    --at) AT="${2:?--at には HH:MM}"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    # 行番号の固定（旧: `2,30p`）はヘッダ長とズレるとコード行まで吐く。実際 #585 以前から
    # `set -euo pipefail` 以降が 10 行漏れていた。アンカーで終端を取り、アンカー自身を落とす。
    -h|--help) sed -n '2,/^set -euo/p' "$0" | sed '$d'; exit 0 ;;
    *) echo "不明な引数: $1" >&2; exit 2 ;;
  esac
done

DATE="${DATE:-$(TZ=Asia/Tokyo date +%Y-%m-%d)}"
[[ "$DATE" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || { echo "DATE は YYYY-MM-DD: $DATE" >&2; exit 2; }
[[ "$BUFFER_MIN" =~ ^[0-9]+$ ]] || { echo "BUFFER_MIN は整数（分）: $BUFFER_MIN" >&2; exit 2; }
if [ -n "$AT" ]; then
  [[ "$AT" =~ ^([0-9]{1,2}):([0-9]{2})$ ]] || { echo "--at は HH:MM: $AT" >&2; exit 2; }
  # 時 0-23・分 0-59 の範囲も検証（Python 側 hhmm_to_min と対称。25:00 等を弾く）。
  { [ "$((10#${BASH_REMATCH[1]}))" -le 23 ] && [ "$((10#${BASH_REMATCH[2]}))" -le 59 ]; } \
    || { echo "--at は 00:00〜23:59: $AT" >&2; exit 2; }
fi

DB_URL="${PADDOCK_DB_URL:-postgres://paddock:paddock@127.0.0.1:5432/paddock}"
WORKDIR="${WORKDIR:-${TMPDIR:-/tmp}/paddock-keep-awake}"
mkdir -p "$WORKDIR/logs"
LOG="$WORKDIR/logs/keep-awake.log"
log() { echo "[$(date '+%Y-%m-%dT%H:%M:%S%z')] $*" | tee -a "$LOG"; }
# エポック秒をログ用の HH:MM へ。BSD date（macOS）と GNU date で -r/-d が異なるため両方試し、
# どちらも駄目なら生の数値を返す（ログ整形のためにスクリプトを止めない）。
fmt_epoch() {
  date -r "$1" '+%H:%M' 2>/dev/null || date -d "@$1" '+%H:%M' 2>/dev/null || echo "$1"
}

# 当日の最終 post_time（HH:MM）を DB から取得。post_time は TEXT 'HH:MM'（ゼロ埋め）なので
# 文字列 MAX で時刻最大＝最終発走になる。post_time NULL は除外。接続不可は中断（無言で
# no-op にしない＝障害を取りこぼし扱いにしない）。
if ! LAST_POST="$(PGCONNECT_TIMEOUT="${PGCONNECT_TIMEOUT:-5}" psql "$DB_URL" -tA -c \
      "SELECT MAX(post_time) FROM race_cards \
       WHERE date='$DATE' AND post_time IS NOT NULL AND post_time ~ '^[0-9]{2}:[0-9]{2}\$';" 2>>"$LOG")"; then
  log "DB から最終 post_time を取得できず中断（接続不可等）"; exit 1
fi
LAST_POST="$(printf '%s' "$LAST_POST" | tr -d '[:space:]')"
if [ -z "$LAST_POST" ]; then
  log "対象なし: $DATE は post_time 入りカードが無い（開催外/未投入）。no-op"; exit 0
fi

# HH:MM → 分。now は --at 優先、無ければ JST 現在時刻。
to_min() { local h="${1%%:*}" m="${1##*:}"; echo $((10#$h * 60 + 10#$m)); }
LAST_MIN="$(to_min "$LAST_POST")"
END_MIN=$((LAST_MIN + BUFFER_MIN))
if [ -n "$AT" ]; then NOW_MIN="$(to_min "$AT")"; else NOW_MIN="$(TZ=Asia/Tokyo date +'%H %M' | awk '{print $1*60+$2}')"; fi

if [ "$NOW_MIN" -ge "$END_MIN" ]; then
  log "発走ウィンドウ終了済み: now=${NOW_MIN} >= end=${END_MIN}（最終 post ${LAST_POST} + buffer ${BUFFER_MIN}分）。no-op"
  exit 0
fi
SECS=$(((END_MIN - NOW_MIN) * 60))

# 抑止終了の**絶対時刻**（エポック秒）。lock に記録して次サイクルの延長判定に使う（#585）。
#
# **分境界へ丸めるのが要点**。素朴に `date +%s + SECS` と書くと、SECS が分粒度（NOW_MIN は秒を
# 切り捨てている）なので END_EPOCH = 真の終了時刻 + 実行時刻の秒針 になる。すると次サイクルの
# 比較 `cur_end >= END_EPOCH` が「前回の秒針 >= 今回の秒針」に退化し、**post_time が全く
# 変わらなくても約半数のサイクルで「延長」と誤判定して健全な caffeinate を張り直す**
# （抑止は切れないが、延長要否の判定そのものが機能しなくなる）。
# エポックを 60 で丸めた値は「現在の分の開始」——JST のオフセットは分単位なので TZ に依らず
# 一致する。これで END_EPOCH は post_time が同じ限り毎サイクル同じ値になり、比較が決定的になる。
END_EPOCH=$(($(date +%s) / 60 * 60 + SECS))

if [ "$DRY_RUN" -eq 1 ]; then
  log "[dry-run] $DATE 最終post=$LAST_POST end=$END_MIN(now=$NOW_MIN) → caffeinate -i -t ${SECS}s"
  exit 0
fi

# caffeinate は macOS 専用。非 macOS や不在環境では何もしない（移設先ホスト等で誤動作させない）。
if ! command -v caffeinate >/dev/null 2>&1; then
  log "caffeinate 不在（非 macOS?）。アイドルスリープ抑止はスキップ"; exit 0
fi

# 多重起動防止: lockdir に稼働中 caffeinate の PID を記録し、生きていれば再起動しない
# （StartInterval で 5 分毎に発火しても caffeinate を積み上げない）。caffeinate は -t で自動終了し
# PID が死ぬと lock は stale 化するので、次回起動時に PID 生存を見て掃除する（self-heal。
# 専用の後始末プロセスは持たない＝兄弟 PID を wait できない罠を避ける）。
# mkdir のアトミック性を排他取得の唯一の門にする（check→rm→mkdir の TOCTOU を避ける）。
# mkdir〜pid 記入の窓でプロセスが死ぬと pid 未記入の空 lock が残りうる。これを「起動中」と取り違えて
# 放置すると keep-awake が恒久的に無言停止するため、mtime の時効（STARTUP_GRACE_MIN 分）で
# 「起動中の正常 lock」と「窓内で死んだ残骸」を見分けて self-heal する。StartInterval(5分) より短くし、
# 最大 1 サイクルの取りこぼしで自己回復させる。
STARTUP_GRACE_MIN=2
# lock は **UID スコープの固定パス**（#643）。
# - `$TMPDIR` は使えない: launchd は TMPDIR を設定しないので `${TMPDIR:-/tmp}` が /tmp に落ち、
#   端末（/var/folders/.../T/）と別の lock を見て互いを見失う（2026-08-19 に実測）。
#   `prefetch_odds.sh` が同じ理由で「lock は WORKDIR に依存させず固定」と決めているのと同じ判断。
# - uid を挟むのは**同一ホストの別ユーザーとの事故衝突を避ける**ため。`id -u` は launchd 経由でも
#   端末でも同じ値に解決される。
#   **悪意ある先回りは防げない**——`/tmp` は world-writable で、sticky bit が禁じるのは他人の
#   エントリの削除・改名だけであり、新しい名前の作成は誰でもできる。他ユーザーが
#   `/tmp/paddock-keep-awake-<uid>.lock.d` を先に作れば mkdir も rm も失敗し続ける。
#   その場合は下の取得失敗ハンドラが所有者を見て**大きく警告する**（無言停止させない）。
# **相方の deployments/launchd/uninstall.sh も同じ式を持つ。片方だけ変えると uninstall が
# caffeinate を止められなくなるので、変えるときは必ず両方を同時に直すこと。**
LOCK_DIR="${PADDOCK_KEEP_AWAKE_LOCK_DIR:-/tmp/paddock-keep-awake-$(id -u).lock.d}"
# 旧パス（uid 無し）。移行期に見て、生きた caffeinate を取りこぼさない（#643）。
# テストが実運用の旧 lock を掴んで本物の caffeinate を殺さないよう、env で注入可能にする。
LEGACY_LOCK_DIR="${PADDOCK_KEEP_AWAKE_LEGACY_LOCK_DIR:-/tmp/paddock-keep-awake.lock.d}"

# 稼働中 caffeinate の pid を引き継ぐ先。空なら「引き継ぐものは無い」。
inherited_pid=""

# 旧パスに生きた caffeinate が居れば pid を引き継ぐ。修正前の launchd が起動した caffeinate が
# 残ったまま新コードへ切り替わると、新 lock からは見えず二重起動になるため。
#
# **ここでは旧ディレクトリを消さない**。消してから early exit（延長不要 / 別プロセス起動中 /
# lock 競合）すると、生きた caffeinate の pid 記録だけが失われ、uninstall からも次サイクルからも
# 止められない孤児になる。削除するのは「引き継いだ pid を確実に停止した後」か「そもそも
# 生存していなかったとき」だけ。
#
# symlink / 他ユーザー所有は触らない——`cat` の中身がそのまま kill の引数になるため。
if [ "$LOCK_DIR" != "$LEGACY_LOCK_DIR" ] && [ -d "$LEGACY_LOCK_DIR" ] \
   && [ ! -L "$LEGACY_LOCK_DIR" ] && [ -O "$LEGACY_LOCK_DIR" ]; then
  legacy_pid="$(cat "$LEGACY_LOCK_DIR/pid" 2>/dev/null || echo '')"
  if [ -n "$legacy_pid" ] && kill -0 "$legacy_pid" 2>/dev/null \
     && ps -p "$legacy_pid" -o comm= 2>/dev/null | grep -q 'caffeinate'; then
    inherited_pid="$legacy_pid"
    log "旧 lock パスの caffeinate を引き継ぐ（pid ${legacy_pid}）: ${LEGACY_LOCK_DIR} → ${LOCK_DIR}"
  else
    # 生存判定まで至らなかった（pid 未記入）ケースと、判定して死んでいたケースを区別する
    # ——障害調査で「生存せず」と書いてあるのに生存判定していない、を避ける。
    if [ -z "$legacy_pid" ]; then
      log "旧 lock パスの残骸を片付ける（pid の記録が無い）"
    else
      log "旧 lock パスの残骸を片付ける（pid=${legacy_pid}・生存せず）"
    fi
    rm -rf "$LEGACY_LOCK_DIR" 2>/dev/null || true
  fi
fi

# 置き換える対象の pid 群（現行 lock 側と旧パス側の両方が生きていることがある）。
# 単一変数に代入すると片方を黙って取りこぼし、落とせなかった方が孤児になる。
superseded_pids=()

if ! mkdir "$LOCK_DIR" 2>/dev/null; then
  # lock 既存。中身で「稼働中／起動中／stale」を見分ける。
  pid="$(cat "$LOCK_DIR/pid" 2>/dev/null || echo '')"
  # 稼働中: pid 生存かつプロセス名が caffeinate（PID 再利用の誤判定を comm で排除）。
  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null \
     && ps -p "$pid" -o comm= 2>/dev/null | grep -q 'caffeinate'; then
    # **窓が足りているかを見る（#585）**。記録された終了時刻が新しい END より後なら据え置き。
    # 手前なら張り直す——朝の install 時点でカードが途中までしか入っていないと窓が短いまま
    # 固定され、caffeinate の自然終了から次の tick まで抑止空白が空く。
    cur_end="$(cat "$LOCK_DIR/end" 2>/dev/null || echo '')"
    # 数値でなければ「読めない」＝安全側（張り直す）へ。`[ x -ge y ]` に非数値を渡すと
    # exit 2 になるので、比較の前に弾いておく。
    [[ "$cur_end" =~ ^[0-9]+$ ]] || cur_end=""
    if [ -n "$cur_end" ] && [ "$cur_end" -ge "$END_EPOCH" ]; then
      log "既に caffeinate 稼働中（pid ${pid}）。抑止終了 $(fmt_epoch "$cur_end") は必要窓 $(fmt_epoch "$END_EPOCH") を満たす。延長不要・据え置き"
      # 据え置いて終わる場合でも、旧パスから引き継いだ caffeinate は始末してから抜ける。
      # 現行 lock の caffeinate が既に窓を握っているので、ここで落としても空白は生まれない。
      # 放置すると旧ディレクトリごと残り、次サイクルも同じ引き継ぎを繰り返す。
      if [ -n "$inherited_pid" ]; then
        kill "$inherited_pid" 2>/dev/null \
          && log "旧パスから引き継いだ余分な caffeinate を停止（pid ${inherited_pid}）" \
          || log "⚠ 旧パスの caffeinate (pid ${inherited_pid}) を停止できず。-t で自然終了するまで二重に抑止が掛かる"
        rm -rf "$LEGACY_LOCK_DIR" 2>/dev/null || true
      fi
      exit 0
    fi
    # end 未記入 / 数値でない（旧形式の lock・書き込み途中）は**延長が必要とみなす**。
    # 判断できないときは抑止を切らさない側へ倒す。
    if [ -z "$cur_end" ]; then
      log "稼働中 caffeinate（pid ${pid}）に終了時刻の記録が無い（旧形式か破損）。安全側に倒して張り直す"
    else
      log "抑止窓を延長する: 現行 $(fmt_epoch "$cur_end") → 必要 $(fmt_epoch "$END_EPOCH")（${DATE} 最終post ${LAST_POST}）"
    fi
    # **新を起動してから旧を落とす**（下の起動ブロックで実施）。逆順にすると kill〜起動の窓で
    # 抑止がゼロになり、いま直そうとしている空白を小さく再現してしまう。
    superseded_pids+=("$pid")
  else
    # pid 未記入かつ lock が新しい（grace 分以内）＝別プロセスが今まさに起動中。掃除せず終了。
    if [ -z "$pid" ] \
       && [ -z "$(find "$LOCK_DIR" -prune -mmin +"$STARTUP_GRACE_MIN" 2>/dev/null)" ]; then
      log "別プロセスが起動中（lock あり・pid 未記入・新しい）。終了"; exit 0
    fi
    # 残るは stale（caffeinate 死亡/PID 再利用、または起動途中で死んだ古い空 lock）。
    log "stale lock を回収して取り直す（pid=${pid:-未記入}）"
  fi
  # 延長・stale とも lock を取り直す。この rm→mkdir は厳密にはアトミックでないが、同時到達で
  # caffeinate が二重起動しても -t で自動終了する無害事象（launchd はジョブを直列化するため
  # 実発生も稀）。門の単純さを優先する。
  rm -rf "$LOCK_DIR" 2>/dev/null || true
  # 取得できない原因を分けて出す。他ユーザーが先回りして同名ディレクトリを作った場合、
  # mkdir も rm も永久に失敗し続けて keep-awake が**無言で停止**する（#264 が最も避けたい状態）。
  # 「競合」と同じ一行で流すと気づけないので、所有者を見て警告を上げる。
  mkdir "$LOCK_DIR" 2>/dev/null || {
    if [ -e "$LOCK_DIR" ] && [ ! -O "$LOCK_DIR" ]; then
      log "⚠ lock パス ${LOCK_DIR} が自分の所有でない（他ユーザーが先回りして作成？）。抑止を掛けられない——放置すると開催日を通してスリープ抑止が効かない"
    else
      log "lock 競合で取得失敗。終了"
    fi
    exit 0
  }
fi

# 旧パスから引き継いだ caffeinate も「置き換える対象」に**足す**（上書きしない）。
# 現行 lock 側と旧パス側の両方が生きている移行期に、片方を取りこぼして孤児にしないため。
[ -n "$inherited_pid" ] && superseded_pids+=("$inherited_pid")

# アイドルスリープを END まで抑止。-t で自動終了するので開放忘れが無い。launchd 経由では plist の
# AbandonProcessGroup=true により、ジョブ主プロセス（本スクリプト）終了後も caffeinate が存続する
# （未設定だと launchd が同一 PGID を kill して即死する。実 launchd で実証済み）。nohup+disown は
# 端末/cron 経路での SIGHUP 巻き添え回避。先に lock を取ってから起動し、起動直後に pid を書く。
nohup caffeinate -i -t "$SECS" >/dev/null 2>&1 &
CAF_PID=$!
echo "$CAF_PID" > "$LOCK_DIR/pid"
# 抑止終了の絶対時刻を併記する（#585）。次サイクルはこれと必要窓を比べて延長要否を決める。
# pid の**後**に書くことで、pid だけある中間状態＝「end 不明」に倒れる（安全側＝張り直す）。
echo "$END_EPOCH" > "$LOCK_DIR/end"
disown 2>/dev/null || true
log "caffeinate -i -t ${SECS}s 起動（pid ${CAF_PID}）。${DATE} 最終post ${LAST_POST} まで抑止（終了 $(fmt_epoch "$END_EPOCH")）"

# **新を起動してから旧を落とす**（#585）。ここまで来た時点で新しい caffeinate が抑止を握って
# いるので、旧を止めても空白が生まれない。ps の comm 照合は「その pid がまだ caffeinate か」の
# 再確認（起動〜ここまでの間に終了して PID が再利用される可能性を潰す）。
# `${arr[@]+...}` は bash 3.2 の `set -u` で空配列の展開が unbound になるのを避ける定型。
for superseded_pid in ${superseded_pids[@]+"${superseded_pids[@]}"}; do
  if kill -0 "$superseded_pid" 2>/dev/null \
     && ps -p "$superseded_pid" -o comm= 2>/dev/null | grep -q 'caffeinate'; then
    if kill "$superseded_pid" 2>/dev/null; then
      log "旧 caffeinate を停止（pid ${superseded_pid}）。窓の張り直し完了"
    else
      # 落とせなくても実害は小さい（旧は -t で自然終了する）。二重に抑止が掛かるだけ。
      log "⚠ 旧 caffeinate (pid ${superseded_pid}) を停止できず。-t で自然終了するまで二重に抑止が掛かる"
    fi
  else
    log "旧 caffeinate (pid ${superseded_pid}) は既に終了していた"
  fi
done

# 引き継ぎ元の旧ディレクトリは、その pid を始末し切った**ここで**消す。早く消すと early exit した
# 経路で生きた caffeinate の記録が失われる（誰も止められない孤児になる）。
if [ -n "$inherited_pid" ]; then
  rm -rf "$LEGACY_LOCK_DIR" 2>/dev/null || true
fi
