#!/usr/bin/env bash
# 条件付き単勝バックテスト用の入力データを生成する（#208）。
#
# 出力先（WORKDIR、既定 /tmp）:
#   bt_races.tsv       レース一覧（win_backtest.py の --races）
#   bt_winodds.tsv     単勝オッズ（--winodds）
#   bt_pred_DATE.txt   analyze predict 出力（--pred-dir）
#   res_NKID.html      netkeiba 結果 HTML（--results-dir）
#
# 使い方:
#   scripts/predict-check/gen_win_backtest_data.sh [WORKDIR]
#
# 環境変数で対象期間・predict 設定を上書きできる（既定は #208 の win-backtest 用）:
#   PADDOCK_BT_FROM   対象開始日（既定 2026-05-30）
#   PADDOCK_BT_TO     対象終了日（既定 2026-06-14）
#   PADDOCK_BT_ALPHA  predict の --blend-alpha（既定 0.3）
#   PADDOCK_ANALYZE_BIN  analyze バイナリのパス（別 worktree のビルドを流用する時など）
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORKDIR="${1:-/tmp}"
DB_URL="${PADDOCK_DB_URL:-postgres://paddock:paddock@127.0.0.1:5432/paddock}"
ANALYZE_BIN="${PADDOCK_ANALYZE_BIN:-$REPO_ROOT/target/release/paddock-analyze}"
FROM="${PADDOCK_BT_FROM:-2026-05-30}"
TO="${PADDOCK_BT_TO:-2026-06-14}"
ALPHA="${PADDOCK_BT_ALPHA:-0.3}"
PSQL=(psql "$DB_URL" -tA)

[[ -x "$ANALYZE_BIN" ]] || {
  echo "release バイナリが見つかりません: $ANALYZE_BIN" >&2
  echo "先に: cd $REPO_ROOT && cargo build --release --bin paddock-analyze" >&2
  exit 1
}

# 日本語場名 → 場コード（netkeiba race_id 構成用）
jp_to_code() {
  case "$1" in
    札幌) echo "01" ;; 函館) echo "02" ;; 福島) echo "03" ;; 新潟) echo "04" ;;
    東京) echo "05" ;; 中山) echo "06" ;; 中京) echo "07" ;; 京都) echo "08" ;;
    阪神) echo "09" ;; 小倉) echo "10" ;;
    *) echo "" ;;
  esac
}

echo "[1/4] レース一覧 (bt_races.tsv)"
"${PSQL[@]}" -F$'\t' -c \
  "SELECT rc.date, rc.race_id, rc.venue, rc.round::text, rc.day::text, rc.race_num::text,
          '--nk--'
   FROM race_cards rc
   WHERE rc.date >= '$FROM' AND rc.date <= '$TO'
   ORDER BY rc.date, rc.venue, rc.race_num;" | \
while IFS=$'\t' read -r date pid venue rnd day rnum _; do
  # '--nk--' は SQL クエリ都合のプレースホルダ; nkid は shell 側で算出する
  vc=$(jp_to_code "$venue")
  [[ -n "$vc" ]] || { echo "未知の場名: $venue" >&2; continue; }
  nkid=$(printf '%s%s%02d%02d%02d' "${pid:0:4}" "$vc" "$rnd" "$day" "$rnum")
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$date" "$pid" "$venue" "$rnd" "$day" "$rnum" "$nkid"
done > "$WORKDIR/bt_races.tsv"
wc -l "$WORKDIR/bt_races.tsv"

echo "[2/4] 単勝オッズ (bt_winodds.tsv)"
"${PSQL[@]}" -F$'\t' -c \
  "SELECT o.race_id, o.combination_key, COALESCE(o.popularity::text, '0'), o.odds::text
   FROM race_odds o
   JOIN race_cards rc ON rc.race_id = o.race_id
   WHERE o.bet_type = 'win'
     AND rc.date >= '$FROM' AND rc.date <= '$TO'
   ORDER BY o.race_id, o.popularity;" > "$WORKDIR/bt_winodds.tsv"
wc -l "$WORKDIR/bt_winodds.tsv"

echo "[3/4] analyze predict（bt_pred_DATE.txt）"
while IFS=$'\t' read -r date pid venue _ _ rnum _; do
  outf="$WORKDIR/bt_pred_${date}.txt"
  # 冪等: ヘッダ AND 馬番行が両方あればスキップ（ヘッダのみの失敗状態は再試行）
  grep -qF "レース ${rnum}: ${venue}" "$outf" 2>/dev/null \
    && grep -qE '^[[:space:]]*[0-9]+' "$outf" 2>/dev/null && continue
  # surface/distance を DB から取得してヘッダを組み立てる
  row=$("${PSQL[@]}" -F$'\t' -c "SELECT surface, distance FROM race_cards WHERE race_id='${pid}';")
  [[ -z "$row" ]] && { echo "  WARN: race_card 未取得 ($pid)" >&2; continue; }
  read -r raw_surf dist <<< "$row"
  case "$raw_surf" in turf) surf=芝 ;; dirt) surf=ダート ;; *) surf="$raw_surf" ;; esac
  # predict 結果を先に取得し、0 行なら書き込まずスキップ
  pred_lines=$("$ANALYZE_BIN" predict "$pid" --blend-alpha "$ALPHA" 2>/dev/null \
    | grep -E '^[[:space:]]*[0-9]+' || true)
  if [[ -z "$pred_lines" ]]; then
    echo "  WARN: predict が空 ($pid)" >&2
    continue
  fi
  echo "  analyze predict $pid"
  echo "--- レース ${rnum}: ${venue} ${surf} ${dist}m ---" >> "$outf"
  printf '%s\n' "$pred_lines" >> "$outf"
done < "$WORKDIR/bt_races.tsv"

echo "[4/4] netkeiba 結果 HTML（res_NKID.html）"
while IFS=$'\t' read -r _ _ _ _ _ _ nkid; do
  outf="$WORKDIR/res_${nkid}.html"
  [[ -f "$outf" ]] && continue
  url="https://race.netkeiba.com/race/result.html?race_id=${nkid}"
  echo "  curl $nkid"
  curl -sf --max-time 20 -H "User-Agent: Mozilla/5.0" --url "$url" -o "$outf" || {
    echo "  FAIL: $nkid" >&2
    rm -f "$outf"
  }
  sleep 1  # netkeiba pacing
done < "$WORKDIR/bt_races.tsv"

echo "done. 出力先: $WORKDIR"
