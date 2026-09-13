#!/usr/bin/env bash
# scripts/hooks/session-stale-check.sh の回帰テスト（#680）。
#
# SessionStart hook は毎回サイレントに実行されるため、壊れて「常に無言」になっても
# 誰も気づけない。bump-distilled-sha.py の出力形式（STALE な文書は無い / （dry-run）行）
# に依存した文字列処理なので、その依存を固定する。
#
# 一時ディレクトリに git repo を作り、スタブの scripts/bump-distilled-sha.py を配置して
# hook を走らせる（hook は `git rev-parse --show-toplevel` で基準を取る）。
set -euo pipefail

# git hook から呼ばれると GIT_DIR が環境に入り、一時リポジトリ内の git が別のリポジトリを
# 指してしまう（test-check-vendored-swagger.sh と同じ罠・#645）。
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE

TARGET=$(cd "$(dirname "$0")" && pwd)/hooks/session-stale-check.sh
pass=0
fail=0

make_repo() {
    # $1 = scripts/bump-distilled-sha.py の中身（省略時は配置しない＝不在ケース）
    local dir
    dir=$(mktemp -d)
    git -C "$dir" init -q
    mkdir -p "$dir/scripts"
    if [ -n "${1:-}" ]; then
        printf '%s' "$1" > "$dir/scripts/bump-distilled-sha.py"
    fi
    echo "$dir"
}

STUB_NO_STALE='#!/usr/bin/env python3
print("STALE な文書は無い")
'

STUB_THREE_STALE='#!/usr/bin/env python3
print("（dry-run）docs/knowledge/a.md")
print("（dry-run）docs/knowledge/b.md")
print("（dry-run）docs/knowledge/c.md")
'

check() {
    # $1 = ケース名 / $2 = stub 内容（空なら不在ケース） / $3 = 期待する出力パターン（grep -F。空なら「出力なし」）
    local name="$1" stub="$2" expect_pattern="$3" dir out code
    dir=$(make_repo "$stub")
    set +e
    out=$(cd "$dir" && bash "$TARGET" 2>&1)
    code=$?
    set -e
    rm -rf "$dir"

    if [ "$code" -ne 0 ]; then
        echo "  ✗ ${name}（exit ${code} が期待の 0 でない）" >&2
        fail=$((fail + 1))
        return
    fi

    if [ -z "$expect_pattern" ]; then
        if [ -z "$out" ]; then
            echo "  ✓ $name"
            pass=$((pass + 1))
        else
            echo "  ✗ ${name}（出力なしを期待したが: ${out}）" >&2
            fail=$((fail + 1))
        fi
        return
    fi

    if echo "$out" | grep -qF "$expect_pattern"; then
        echo "  ✓ $name"
        pass=$((pass + 1))
    else
        echo "  ✗ ${name}（期待パターン未検出: ${expect_pattern} / 実際: ${out}）" >&2
        fail=$((fail + 1))
    fi
}

echo "session-stale-check.sh 回帰テスト"

check "stale 0 件 → 出力なし" "$STUB_NO_STALE" ""

check "stale 3 件 → 件数表示" "$STUB_THREE_STALE" "⚠ 蒸留が必要な knowledge: 3 件"

check "bump-distilled-sha.py 不在 → グレースフル終了" "" ""

echo
if [ "$fail" -ne 0 ]; then
    echo "✗ $fail / $((pass + fail)) 件が失敗した" >&2
    exit 1
fi
echo "✓ 全 $pass ケース通過"
