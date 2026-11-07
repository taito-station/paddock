#!/bin/bash
# SessionStart hook: knowledge の stale 状態を報告する（#27 タスク 4）。
#
# `bump-distilled-sha.py --all-stale --dry-run` を実行し、STALE が 0 件なら何も出さない
# （ノイズにしない）。1 件以上あれば件数と対象文書を短く出し、hve-akm での蒸留し直しへ
# 案内する（SessionStart の標準出力は Claude の文脈に入る）。`--all-stale` の追従は sha を
# 外形的に揃えるだけで本文を直さないので、stale の案内には使わない。
# bump が判定不能（exit 2）のときはその旨を 1 行だけ出す。STALE が 0 件でも bump が他の
# error で exit 1 のときは、その error が黙って握りつぶされないよう 1 行だけ出す。
#
# python3 が無い・git リポジトリでない・bump 自体が無い／失敗した場合は、何も壊さず exit 0
# にする（セッション開始を妨げない）。
#
# bump のパスは**このスクリプト自身の位置から**解決する。導入先では
# `.claude/scripts/hve/hooks/session-stale-check.sh` に配置され、bump は 1 階層上の
# `.claude/scripts/hve/bump-distilled-sha.py` に同居する前提。
#
# settings.json への配線例（導入先が手で追加する。timeout は 30 秒程度を目安にする。履歴を遡るので大きなリポジトリでは時間がかかる）:
#   {
#     "hooks": {
#       "SessionStart": [
#         {
#           "hooks": [
#             {
#               "type": "command",
#               "command": "bash \"$CLAUDE_PROJECT_DIR\"/.claude/scripts/hve/hooks/session-stale-check.sh",
#               "timeout": 30
#             }
#           ]
#         }
#       ]
#     }
#   }
# このスクリプトが受け取った引数は、そのまま bump の `--` 以降（checker への転送引数）に渡る。
# checker の既定を変える場合は command にそのまま続けて書く（例）:
#   "command": "bash \"$CLAUDE_PROJECT_DIR\"/.claude/scripts/hve/hooks/session-stale-check.sh --required status,kind,sources,distilled_from_sha,updated --allow-empty-sources-with-decision-log",
set -uo pipefail

HOOK_DIR="$(cd "$(dirname "$0")" && pwd)"
BUMP="$HOOK_DIR/../bump-distilled-sha.py"

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0
command -v python3 >/dev/null 2>&1 || exit 0
[ -f "$BUMP" ] || exit 0

CHECK_EXAMPLE="python3 .claude/scripts/hve/check-knowledge.py"
if [ "$#" -gt 0 ]; then
  CHECK_EXAMPLE="$CHECK_EXAMPLE $*"
fi

output="$(cd "$REPO_ROOT" && python3 "$BUMP" --all-stale --dry-run -- "$@" 2>&1)"
rc=$?

if [ "$rc" -eq 2 ]; then
  echo "⚠ knowledge の stale 判定が完了できない（判定不能）。bump-distilled-sha.py の出力を確認する"
  exit 0
fi

# 行全体が一致するときだけ「STALE 無し」と見なす（G9）。bump の終了コードが 0 の
# ときに限定するのは、frontmatter の値に「STALE な文書は無い」という文言が紛れ込み、
# 他の error 行の一部として出力されても、grep の部分一致で通知を握りつぶさないため。
if [ "$rc" -eq 0 ] && echo "$output" | grep -qx "STALE な文書は無い"; then
  exit 0
fi

stale_count=$(echo "$output" | grep -c "^（dry-run）" || true)
if [ "$stale_count" -gt 0 ]; then
  echo "⚠ 蒸留が必要な knowledge: ${stale_count} 件"
  echo "  hve-akm で蒸留し直す（source の差分を本文にマージしてから bump する）"
  echo "  確認: ${CHECK_EXAMPLE}"
  echo "$output" | sed -n 's/^（dry-run）\([^ ]*\) →.*/  - \1/p'
elif [ "$rc" -eq 1 ]; then
  # STALE 行は 0 本だが bump が exit 1。checker に STALE 以外の error が残っている
  # （stale_count だけを見ると何も出さず、この error を握りつぶしてしまう）。
  echo "⚠ knowledge の検査に STALE 以外の error がある。\`${CHECK_EXAMPLE}\` で確認する"
fi

exit 0
