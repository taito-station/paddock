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
#   PADDOCK_DB_URL  Postgres 接続 URL（既定: postgres://paddock:paddock@localhost:5432/paddock）
#   WORKDIR         中間 TSV の出力先（既定: $TMPDIR/paddock-live-ev）
set -euo pipefail

DATE="${1:?usage: refresh_ev.sh <YYYY-MM-DD> [first_R] [last_R] [budget]}"
FIRST_R="${2:-6}"
LAST_R="${3:-12}"
BUDGET="${4:-5000}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DB_URL="${PADDOCK_DB_URL:-postgres://paddock:paddock@localhost:5432/paddock}"
WORKDIR="${WORKDIR:-${TMPDIR:-/tmp}/paddock-live-ev}"
mkdir -p "$WORKDIR"
PSQL=(psql "$DB_URL" -tA)

cd "$REPO_ROOT"

# paddock race_id（例 2026-3-tokyo-5-6R）→ netkeiba 12桁 race_id を構成。
nk_id() {
  python3 - "$1" <<'PY'
import sys
pid = sys.argv[1]
_, kai, ven, day, rr = pid.split("-")
vc = {"sapporo":"01","hakodate":"02","fukushima":"03","niigata":"04","tokyo":"05",
      "nakayama":"06","chukyo":"07","kyoto":"08","hanshin":"09","kokura":"10"}[ven]
print(f"2026{vc}{int(kai):02d}{int(day):02d}{int(rr.rstrip('R')):02d}")
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

echo "[1/5] fetch-card --force（netkeiba 最新オッズ → Postgres） ${#PIDS[@]} レース"
for pid in "${PIDS[@]}"; do
  nk="$(nk_id "$pid")"
  if cargo run -q --bin paddock-fetch-card -- "$nk" --force --skip-history --interval 800 \
       >/dev/null 2>&1; then echo "  ok   $pid ($nk)"; else echo "  FAIL $pid ($nk)"; fi
  sleep 1
done

echo "[2/5] horses TSV"
"${PSQL[@]}" -F$'\t' -c \
  "SELECT e.race_id, e.horse_num, e.horse_name, COALESCE(e.jockey,''), \
          COALESCE(o.popularity,99), COALESCE(o.odds,0) \
   FROM horse_entries e \
   LEFT JOIN race_odds o ON o.race_id=e.race_id AND o.bet_type='win' \
        AND o.combination_key=e.horse_num::text \
   JOIN race_cards c ON c.race_id=e.race_id \
   WHERE c.date='$DATE' AND c.race_num BETWEEN $FIRST_R AND $LAST_R \
   ORDER BY e.race_id, e.horse_num;" > "$WORKDIR/horses.tsv"

echo "[3/5] exotic TSV（馬連/3連複）"
"${PSQL[@]}" -F$'\t' -c \
  "SELECT race_id, bet_type, combination_key, odds FROM race_odds o \
   WHERE bet_type IN ('quinella','trio') \
     AND race_id IN (SELECT race_id FROM race_cards \
                     WHERE date='$DATE' AND race_num BETWEEN $FIRST_R AND $LAST_R) \
   ORDER BY race_id, bet_type, combination_key;" > "$WORKDIR/exotic.tsv"

echo "[4/5] wide TSV（netkeiba type=5）"
: > "$WORKDIR/wide.tsv"
for pid in "${PIDS[@]}"; do
  python3 "$SCRIPT_DIR/fetch_wide.py" "$(nk_id "$pid")" "$pid" >> "$WORKDIR/wide.tsv" 2>/dev/null || true
  sleep 1
done

echo "[5/5] meta + 予想（analyze predict --blend-alpha 0.3）"
: > "$WORKDIR/meta.tsv"
: > "$WORKDIR/pred.txt"
"${PSQL[@]}" -F$'\t' -c \
  "SELECT race_id, venue, race_num, surface, distance FROM race_cards \
   WHERE date='$DATE' AND race_num BETWEEN $FIRST_R AND $LAST_R \
   ORDER BY venue, race_num;" | \
while IFS=$'\t' read -r pid venue rnum surf dist; do
  [ -n "$pid" ] || continue
  printf '%s\t%s\t%s\n' "$pid" "$venue" "$rnum" >> "$WORKDIR/meta.tsv"
  jsurf=$([ "$surf" = "turf" ] && echo "芝" || echo "ダート")
  echo "--- レース $rnum: $venue $jsurf ${dist}m ---" >> "$WORKDIR/pred.txt"
  cargo run -q --bin paddock-analyze -- predict "$pid" --blend-alpha 0.3 2>/dev/null \
    | grep -E '^[[:space:]]*[0-9]+[[:space:]]' >> "$WORKDIR/pred.txt" || true
  echo >> "$WORKDIR/pred.txt"
done

echo "=== EV ==="
python3 "$SCRIPT_DIR/live_ev.py" \
  --pred "$WORKDIR/pred.txt" --meta "$WORKDIR/meta.tsv" --horses "$WORKDIR/horses.tsv" \
  --exotic "$WORKDIR/exotic.tsv" --wide "$WORKDIR/wide.tsv" --budget "$BUDGET" --slip
