#!/usr/bin/env bash
# 条件付き単勝バックテスト用の入力データを生成する（#208）。
#
# 出力先（WORKDIR、既定 /tmp）:
#   bt_races.tsv        レース一覧（win_backtest.py の --races）
#   bt_winodds.tsv      単勝オッズ（--winodds）
#   bt_exotic_odds.tsv  エキゾオッズ 馬連/3連複/馬単（exotic_mispricing.py の --exotic-odds, #314）
#   bt_pred_DATE.txt    analyze predict 出力（--pred-dir）
#   res_NKID.html       netkeiba 結果 HTML（--results-dir）
#
# 使い方:
#   scripts/predict-check/gen_win_backtest_data.sh [WORKDIR]
#
# 環境変数で対象期間・predict 設定を上書きできる（FROM/TO の既定は #208 の win-backtest 窓）:
#   PADDOCK_BT_FROM   対象開始日（既定 2026-05-30）
#   PADDOCK_BT_TO     対象終了日（既定 2026-06-14）
#   PADDOCK_BT_ALPHA  predict の --blend-alpha（既定 0.2 ＝本番モデル, ADR 0034。
#                     #208 当時の α=0.3 を再現するなら PADDOCK_BT_ALPHA=0.3 を渡す）
#   PADDOCK_BT_SHRINKAGE_M  predict の --shrinkage-m（#282）。未設定なら本番既定 m=10。
#                     #270/ADR 0045 の m×α×γ 再検証で m を振った α=1.0 bt_pred を作るとき、m 値ごとに
#                     別 WORKDIR へ本スクリプトを回す（例: PADDOCK_BT_ALPHA=1.0 PADDOCK_BT_SHRINKAGE_M=20）。
#                     γ（win_power）は本番既定 1.25 固定で生成する（受け口を出さない）。umaren_backtest.py の
#                     recover_p_models が γ=1.25 で逆変換するため、ここで γ を変えると p_model 復元が狂う。
#   PADDOCK_ANALYZE_BIN  analyze バイナリのパス（別 worktree のビルドを流用する時など）
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORKDIR="${1:-/tmp}"
DB_URL="${PADDOCK_DB_URL:-postgres://paddock:paddock@127.0.0.1:5432/paddock}"
ANALYZE_BIN="${PADDOCK_ANALYZE_BIN:-$REPO_ROOT/target/release/paddock-analyze}"
FROM="${PADDOCK_BT_FROM:-2026-05-30}"
TO="${PADDOCK_BT_TO:-2026-06-14}"
ALPHA="${PADDOCK_BT_ALPHA:-0.2}"
SHRINKAGE_M="${PADDOCK_BT_SHRINKAGE_M:-}"
PSQL=(psql "$DB_URL" -tA)

# FROM/TO は SQL に文字列補間するため、日付形式（数字とハイフンのみ）に制限して注入を防ぐ。
for v in FROM TO; do
  [[ "${!v}" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || { echo "不正な日付 $v=${!v}（YYYY-MM-DD のみ可）" >&2; exit 1; }
done

# ALPHA はブレンド係数 [0,1]。形式と値域を検証する（refresh_ev.sh の LIVE_BLEND_ALPHA と対称）。
[[ "$ALPHA" =~ ^[0-9]+(\.[0-9]+)?$ ]] \
  && LC_ALL=C awk -v a="$ALPHA" 'BEGIN{exit !(a>=0 && a<=1)}' \
  || { echo "PADDOCK_BT_ALPHA は 0〜1 の数値: $ALPHA" >&2; exit 1; }

# SHRINKAGE_M（#282）: 未設定なら本番既定 m=10（predict にフラグを渡さない）。設定時は正数のみ許可
# （analyze predict の --shrinkage-m の finite-positive 制約に整合。指数表記や `.5`/`5.` は shell 側で
# 弾く安全側の差はある）。set -u 下でも空配列展開が安全な idiom で predict に渡す。
SHRINKAGE_M_ARGS=()
if [[ -n "$SHRINKAGE_M" ]]; then
  [[ "$SHRINKAGE_M" =~ ^[0-9]+(\.[0-9]+)?$ ]] \
    && LC_ALL=C awk -v m="$SHRINKAGE_M" 'BEGIN{exit !(m>0)}' \
    || { echo "PADDOCK_BT_SHRINKAGE_M は正の数値: $SHRINKAGE_M" >&2; exit 1; }
  SHRINKAGE_M_ARGS=(--shrinkage-m "$SHRINKAGE_M")
fi

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

echo "[1/5] レース一覧 (bt_races.tsv)"
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

echo "[2/5] 単勝オッズ (bt_winodds.tsv)"
"${PSQL[@]}" -F$'\t' -c \
  "SELECT o.race_id, o.combination_key, COALESCE(o.popularity::text, '0'), o.odds::text
   FROM race_odds o
   JOIN race_cards rc ON rc.race_id = o.race_id
   WHERE o.bet_type = 'win'
     AND rc.date >= '$FROM' AND rc.date <= '$TO'
   ORDER BY o.race_id, o.popularity;" > "$WORKDIR/bt_winodds.tsv"
wc -l "$WORKDIR/bt_winodds.tsv"

# エキゾ（馬連/3連複/馬単）オッズ（#314 ミスプライス検証の --exotic-odds 入力）。refresh_ev.sh の
# exotic TSV と同じ列（race_id / bet_type / combination_key / odds）。ワイドは過去データ不足で除外。
echo "[3/5] エキゾオッズ (bt_exotic_odds.tsv)"
"${PSQL[@]}" -F$'\t' -c \
  "SELECT o.race_id, o.bet_type, o.combination_key, o.odds::text
   FROM race_odds o
   JOIN race_cards rc ON rc.race_id = o.race_id
   WHERE o.bet_type IN ('quinella', 'trio', 'exacta')
     AND rc.date >= '$FROM' AND rc.date <= '$TO'
   ORDER BY o.race_id, o.bet_type, o.combination_key;" > "$WORKDIR/bt_exotic_odds.tsv"
wc -l "$WORKDIR/bt_exotic_odds.tsv"

echo "[4/5] analyze predict（bt_pred_DATE.txt）"
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
  pred_lines=$("$ANALYZE_BIN" predict "$pid" --blend-alpha "$ALPHA" \
    ${SHRINKAGE_M_ARGS[@]+"${SHRINKAGE_M_ARGS[@]}"} 2>/dev/null \
    | grep -E '^[[:space:]]*[0-9]+' || true)
  if [[ -z "$pred_lines" ]]; then
    echo "  WARN: predict が空 ($pid)" >&2
    continue
  fi
  echo "  analyze predict $pid"
  echo "--- レース ${rnum}: ${venue} ${surf} ${dist}m ---" >> "$outf"
  printf '%s\n' "$pred_lines" >> "$outf"
done < "$WORKDIR/bt_races.tsv"

echo "[5/5] netkeiba 結果 HTML（res_NKID.html）"
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
