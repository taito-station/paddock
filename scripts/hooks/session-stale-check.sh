#!/bin/bash
# SessionStart hook: knowledge の stale 状態を報告する
# stale が 0 件なら何も出さない（ノイズにしない）
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || echo "$(dirname "$0")/../..")"

output=$(python3 "$REPO_ROOT/scripts/bump-distilled-sha.py" --all-stale --dry-run 2>&1) || true

if echo "$output" | grep -q "STALE な文書は無い"; then
  exit 0
fi

stale_count=$(echo "$output" | grep -c "^STALE:" || true)
if [ "$stale_count" -gt 0 ]; then
  echo "⚠ 蒸留が必要な knowledge: ${stale_count} 件（/akm で解消）"
  echo "$output" | grep "^STALE:" | sed 's/^STALE: /  - /'
fi
