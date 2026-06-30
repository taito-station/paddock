#!/usr/bin/env bash
# 学習型モデル評価ハーネス③ 忠実性ゲート（#272 / #309）のワンコマンド実行。
#
# 指定期間で `analyze backtest` を production 構成で走らせ、--dump-features の TSV を
# faithfulness.py で集計し、backtest 本体の出力と一致することを検証する。終了コードで
# 合否を返す（不一致なら 1）。CI / 回帰の入口に使う。
#
# 使い方:
#   scripts/harness/check_faithfulness.sh <FROM> <TO> [追加 analyze フラグ...]
#   例: scripts/harness/check_faithfulness.sh 2026-06-13 2026-06-14
#
# production 構成（[[project_backtest_production_flags]] と一致）は既定で付与する。
set -euo pipefail

if [ "$#" -lt 2 ]; then
    echo "usage: $0 <FROM> <TO> [extra analyze flags...]" >&2
    exit 2
fi
FROM="$1"
TO="$2"
shift 2

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

DUMP="$(mktemp -t paddock_dump.XXXXXX.tsv)"
REPORT="$(mktemp -t paddock_bt.XXXXXX.txt)"
trap 'rm -f "$DUMP" "$REPORT"' EXIT

# production 構成: m=10 / win_power=1.25 / place_show_power=2.0 / α=0.2。
cargo run -q --manifest-path "$REPO_ROOT/Cargo.toml" -p analyze -- backtest \
    --from "$FROM" --to "$TO" \
    --shrinkage-m 10 --win-power 1.25 --place-show-power 2.0 --blend-alpha 0.2 \
    --dump-features "$DUMP" "$@" 2>/dev/null | tee "$REPORT"

echo
python3 "$SCRIPT_DIR/faithfulness.py" "$DUMP" --backtest-report "$REPORT"
