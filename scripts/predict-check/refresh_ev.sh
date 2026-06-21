#!/usr/bin/env bash
# ライブ EV 更新 — 当日レースの最新オッズを取得し直して期待回収率(ROI)を再計算する.
#
# 用途: 開催当日に 15 分間隔等で回し、+EV(ROI>=100%)のレースを見つける（[[feedback-betting-staking]]）。
# 本体 `fetch-card`(netkeiba→Postgres) + `analyze predict`(最新オッズ込み model 勝率) +
# netkeiba ワイド(type=5, fetch_wide.py) を組み合わせ、live_ev.py で全3券種 ROI を出す。
#
# 使い方:
#   scripts/predict-check/refresh_ev.sh <YYYY-MM-DD> [first_R] [last_R] [budget]
#   例:  scripts/predict-check/refresh_ev.sh 2026-06-20 6 12 5000
#
# 環境変数:
#   PADDOCK_DB_URL   Postgres 接続 URL（既定: postgres://paddock:paddock@localhost:5432/paddock）
#   WORKDIR          中間 TSV の出力先（既定: $TMPDIR/paddock-live-ev）
#   LIVE_WINDOW_MIN  設定すると発走時刻フィルタを有効化し、netkeiba 発走時刻で「これから発走する
#                    かつ発走まで N 分以内」のレースだけを対象にする（#197, 朝の無駄打ち抑制）。
#                    未設定なら R 範囲の全レースを対象（後方互換）。例: LIVE_WINDOW_MIN=60
set -euo pipefail

DATE="${1:?usage: refresh_ev.sh <YYYY-MM-DD> [first_R] [last_R] [budget]}"
FIRST_R="${2:-6}"
LAST_R="${3:-12}"
BUDGET="${4:-5000}"

# 引数は psql の SQL に文字列展開するため、形式を検証して注入・誤クエリを防ぐ。
[[ "$DATE" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || { echo "DATE は YYYY-MM-DD 形式: $DATE" >&2; exit 2; }
[[ "$FIRST_R" =~ ^[0-9]+$ && "$LAST_R" =~ ^[0-9]+$ ]] || { echo "R 範囲は整数: $FIRST_R $LAST_R" >&2; exit 2; }
[[ "$BUDGET" =~ ^[0-9]+$ ]] || { echo "予算は整数（円）: $BUDGET" >&2; exit 2; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# release バイナリの存在確認。debug ビルドでのライブ運用を防ぐ (#211)。
FETCH_BIN="$REPO_ROOT/target/release/paddock-fetch-card"
ANALYZE_BIN="$REPO_ROOT/target/release/paddock-analyze"
_bin_missing=0
[[ -x "$FETCH_BIN" ]]   || { echo "release バイナリが見つかりません: $FETCH_BIN" >&2; _bin_missing=1; }
[[ -x "$ANALYZE_BIN" ]] || { echo "release バイナリが見つかりません: $ANALYZE_BIN" >&2; _bin_missing=1; }
if (( _bin_missing )); then
  echo "先に以下を実行してください:" >&2
  echo "  cd $REPO_ROOT && cargo build --release --bin paddock-fetch-card --bin paddock-analyze" >&2
  exit 1
fi
unset _bin_missing

DB_URL="${PADDOCK_DB_URL:-postgres://paddock:paddock@localhost:5432/paddock}"
WORKDIR="${WORKDIR:-${TMPDIR:-/tmp}/paddock-live-ev}"
mkdir -p "$WORKDIR/logs"
# 実行ごとにログをリセット（累積防止）
> "$WORKDIR/logs/fetch-card.log"
> "$WORKDIR/logs/analyze.log"
PSQL=(psql "$DB_URL" -tA)

cd "$REPO_ROOT"

# paddock race_id（例 2026-3-tokyo-5-6R）→ netkeiba 12桁 race_id を構成。
# 本体に正本 `netkeiba_race_id_from_paddock`(src/use-case/src/netkeiba_race_id.rs) があるが
# CLI 露出が無いため、当スクリプト内で同等の slug→場コード変換を行う（年は pid から取る）。
nk_id() {
  python3 - "$1" <<'PY'
import sys
pid = sys.argv[1]
parts = pid.split("-")  # paddock race_id: {年}-{回}-{場slug}-{日}-{R}R
if len(parts) != 5:
    sys.exit(f"nk_id: 想定外の race_id 形式: {pid}")
year, kai, ven, day, rr = parts
vc = {"sapporo": "01", "hakodate": "02", "fukushima": "03", "niigata": "04", "tokyo": "05",
      "nakayama": "06", "chukyo": "07", "kyoto": "08", "hanshin": "09", "kokura": "10"}.get(ven)
if vc is None:
    # 未知の場 slug は fail-fast（中央10場以外＝想定外データ。黙ってスキップせず止める）。
    sys.exit(f"nk_id: 未知の場 slug: {ven}（pid={pid}）")
# netkeiba race_id = 年 + 場(2) + 回(2) + 日(2) + R(2)。年は決め打ちせず pid から導出する。
print(f"{year}{vc}{int(kai):02d}{int(day):02d}{int(rr.rstrip('R')):02d}")
PY
}

# 対象 paddock race_id（場×R 範囲）。macOS の Bash 3.2 に mapfile が無いため while-read で読む。
PIDS=()
while IFS= read -r line; do
  [ -n "$line" ] && PIDS+=("$line")
done < <("${PSQL[@]}" -c \
  "SELECT race_id FROM race_cards WHERE date='$DATE' \
   AND race_num BETWEEN $FIRST_R AND $LAST_R ORDER BY venue, race_num;")
[ "${#PIDS[@]}" -gt 0 ] || { echo "対象レースなし: $DATE $FIRST_R-${LAST_R}R" >&2; exit 1; }

# 発走時刻ウィンドウ絞り込み（#197, opt-in）。LIVE_WINDOW_MIN が設定されていれば、
# netkeiba の発走時刻を使って「発走済み（now 超過）」と「発走まで window 分より先」を
# 落とし、これから発走する直近レースだけを対象にする（朝の無駄打ち＝JRA 過剰アクセス回避,
# feedback_jra_fetch_pacing）。未設定なら従来どおり R 範囲の全レースを対象にする（後方互換）。
if [ -n "${LIVE_WINDOW_MIN:-}" ]; then
  [[ "$LIVE_WINDOW_MIN" =~ ^[0-9]+$ ]] || { echo "LIVE_WINDOW_MIN は整数（分）: $LIVE_WINDOW_MIN" >&2; exit 2; }
  # upcoming_races.py の出力（3 列目=paddock race_id）を対象集合として読み、PIDS と積を取る。
  # 同 py は nk を同ディレクトリから import するため PYTHONPATH を SCRIPT_DIR に通す。
  # python の終了コードを cut|tr パイプに飲まれないよう一旦変数に受けて成否を判定する。
  # 取得失敗（netkeiba エラー等）を空集合として握り潰すと「全レース終了」と区別できず
  # 古いオッズで EV を誤判定するため、明示的に中断する。
  if ! UPCOMING_RAW="$(PYTHONPATH="$SCRIPT_DIR" python3 "$SCRIPT_DIR/upcoming_races.py" \
                        "${DATE//-/}" --window-min "$LIVE_WINDOW_MIN")"; then
    echo "発走時刻の取得に失敗（netkeiba 取得エラー等）。EV 誤判定回避のため中断する。" >&2
    exit 3
  fi
  UPCOMING=" $(printf '%s\n' "$UPCOMING_RAW" | cut -f3 | tr '\n' ' ') "
  FILTERED=()
  for pid in "${PIDS[@]}"; do
    [[ "$UPCOMING" == *" $pid "* ]] && FILTERED+=("$pid")
  done
  # 空配列の `"${FILTERED[@]}"` 展開は macOS Bash 3.2 + set -u で unbound エラーになり、
  # 直後の親切メッセージに到達せず即死する（朝イチ＝窓内に未発走レース無しが主要ケース）。
  # 代入前に件数で判定して安全に分岐する。
  [ "${#FILTERED[@]}" -gt 0 ] || {
    echo "対象レースなし: $DATE 発走 ${LIVE_WINDOW_MIN} 分以内の未発走レースは無し（全レース終了 or 開催前）" >&2
    exit 1
  }
  PIDS=("${FILTERED[@]}")
  echo "発走時刻フィルタ: 発走 ${LIVE_WINDOW_MIN} 分以内の未発走 ${#PIDS[@]} レースに絞り込み"
fi

# 下流 SQL（horses/exotic/meta）の対象レース述語。LIVE_WINDOW_MIN で絞った場合は
# fetch-card/wide と同じ PIDS 集合に EV 出力も揃える。揃えないと窓外レースが「古い DB
# オッズ＋ワイド欠落」のまま EV に混ざり、「対象を絞る」意図と乖離する（Reviewer 指摘）。
# 未設定時は従来どおり R 範囲 BETWEEN（全レース）。alias 有無で 2 種の述語を用意する。
if [ -n "${LIVE_WINDOW_MIN:-}" ]; then
  # PIDS は race_cards.race_id 由来（DB 取得・検証済み）なので IN リストへ展開して安全。
  RACE_ID_CSV=""
  for pid in "${PIDS[@]}"; do RACE_ID_CSV="${RACE_ID_CSV:+$RACE_ID_CSV,}'$pid'"; done
  RACE_PRED_C="c.race_id IN ($RACE_ID_CSV)"
  RACE_PRED_BARE="race_id IN ($RACE_ID_CSV)"
else
  RACE_PRED_C="c.race_num BETWEEN $FIRST_R AND $LAST_R"
  RACE_PRED_BARE="race_num BETWEEN $FIRST_R AND $LAST_R"
fi

echo "[1/5] fetch-card --force（netkeiba 最新オッズ → Postgres） ${#PIDS[@]} レース"
FETCH_FAILED=()  # 取得失敗レースを集計し、古いオッズでの EV 誤判定を末尾で警告する
for pid in "${PIDS[@]}"; do
  nk="$(nk_id "$pid")"
  if "$FETCH_BIN" "$nk" --force --skip-history --interval 800 \
       > /dev/null 2>> "$WORKDIR/logs/fetch-card.log"; then echo "  ok   $pid ($nk)"
  else echo "  FAIL $pid ($nk)"; FETCH_FAILED+=("$pid"); fi
  sleep 1  # netkeiba への pacing（fetch-card の --interval とは別にループ間隔を空ける）
done

echo "[2/5] horses TSV"
"${PSQL[@]}" -F$'\t' -c \
  "SELECT e.race_id, e.horse_num, e.horse_name, COALESCE(e.jockey,''), \
          COALESCE(o.popularity,99), COALESCE(o.odds,0) \
   FROM horse_entries e \
   LEFT JOIN race_odds o ON o.race_id=e.race_id AND o.bet_type='win' \
        AND o.combination_key=e.horse_num::text \
   JOIN race_cards c ON c.race_id=e.race_id \
   WHERE c.date='$DATE' AND $RACE_PRED_C \
   ORDER BY e.race_id, e.horse_num;" > "$WORKDIR/horses.tsv"

echo "[3/5] exotic TSV（馬連/3連複）"
"${PSQL[@]}" -F$'\t' -c \
  "SELECT o.race_id, o.bet_type, o.combination_key, o.odds FROM race_odds o \
   JOIN race_cards c ON c.race_id=o.race_id \
   WHERE c.date='$DATE' AND $RACE_PRED_C \
     AND o.bet_type IN ('quinella','trio') \
   ORDER BY o.race_id, o.bet_type, o.combination_key;" > "$WORKDIR/exotic.tsv"

echo "[4/5] wide TSV（netkeiba type=5）"
: > "$WORKDIR/wide.tsv"
: > "$WORKDIR/wide_errors.log"
for pid in "${PIDS[@]}"; do
  # 取得失敗は無言で捨てず、stderr をログに残しつつ FAIL を可視化する（ワイド欠落は EV を歪めるため）。
  python3 "$SCRIPT_DIR/fetch_wide.py" "$(nk_id "$pid")" "$pid" >> "$WORKDIR/wide.tsv" \
    2>>"$WORKDIR/wide_errors.log" || echo "  wide FAIL $pid（詳細: $WORKDIR/wide_errors.log）" >&2
  sleep 1  # netkeiba への pacing
done

echo "[5/5] meta + 予想（analyze predict --blend-alpha 0.3）"
: > "$WORKDIR/meta.tsv"
: > "$WORKDIR/pred.txt"
"${PSQL[@]}" -F$'\t' -c \
  "SELECT race_id, venue, race_num, surface, distance FROM race_cards \
   WHERE date='$DATE' AND $RACE_PRED_BARE \
   ORDER BY venue, race_num;" | \
while IFS=$'\t' read -r pid venue rnum surf dist; do
  [ -n "$pid" ] || continue
  printf '%s\t%s\t%s\n' "$pid" "$venue" "$rnum" >> "$WORKDIR/meta.tsv"
  jsurf=$([ "$surf" = "turf" ] && echo "芝" || echo "ダート")
  echo "--- レース $rnum: $venue $jsurf ${dist}m ---" >> "$WORKDIR/pred.txt"
  "$ANALYZE_BIN" predict "$pid" --blend-alpha 0.3 2>> "$WORKDIR/logs/analyze.log" \
    | grep -E '^[[:space:]]*[0-9]+[[:space:]]' >> "$WORKDIR/pred.txt" || true
  echo >> "$WORKDIR/pred.txt"
done

if [ "${#FETCH_FAILED[@]}" -gt 0 ]; then
  echo "⚠ fetch-card 失敗 ${#FETCH_FAILED[@]} 本: ${FETCH_FAILED[*]}" >&2
  echo "   → 該当レースは古い DB オッズで評価される。EV の信頼度が低い点に注意。" >&2
fi
[[ -s "$WORKDIR/logs/fetch-card.log" ]] && \
  echo "⚠ fetch-card stderr あり（詳細: $WORKDIR/logs/fetch-card.log）" >&2
[[ -s "$WORKDIR/logs/analyze.log" ]] && \
  echo "⚠ analyze stderr あり（詳細: $WORKDIR/logs/analyze.log）" >&2

echo "=== EV ==="
python3 "$SCRIPT_DIR/live_ev.py" \
  --pred "$WORKDIR/pred.txt" --meta "$WORKDIR/meta.tsv" --horses "$WORKDIR/horses.tsv" \
  --exotic "$WORKDIR/exotic.tsv" --wide "$WORKDIR/wide.tsv" --budget "$BUDGET" --slip
