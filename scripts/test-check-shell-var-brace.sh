#!/usr/bin/env bash
# scripts/check-shell-var-brace.sh の回帰テスト（#636）。
#
# **なぜ検査にテストが要るか**: リポジトリは常に合格側なので、検査が壊れても本番データが正常だと
# 無言で緑になる。ADR 0073 が `adr` ジョブを「回帰テスト → 本番検査」の順にしたのと同じ理由で、
# fail-closed を謳う検査ほど自身の回帰を持つ必要がある。
#
# 一時 git リポジトリに 1 本の .sh を置いて検査を走らせる（検査は git ls-files で対象を列挙するので
# git init と git add が要る）。**実行時の bash の挙動には依存させない**——本件はロケール依存で
# プラットフォームごとに再現性が変わるため、検査も回帰も字面の静的判定だけで完結させる。
#
# **フィクスチャの `$` は変数経由で組み立てる**（下の DOLLAR）。悪い例をリテラルで書くと、
# このファイル自身が検査に引っかかる。検査側にテストファイルの除外を入れると
# 「検査が自分自身に適用されない」穴になるので、書く側で回避する。
set -euo pipefail

TARGET=$(cd "$(dirname "$0")" && pwd)/check-shell-var-brace.sh
DOLLAR='$'
pass=0
fail=0

check() {
    # $1 = ケース名 / $2 = 期待（pass|fail）/ $3 = スクリプト本文
    local name="$1" expect="$2" body="$3" dir code
    dir=$(mktemp -d)
    git -C "${dir}" init -q
    printf '%s\n' "${body}" > "${dir}/sample.sh"
    git -C "${dir}" add sample.sh
    set +e
    (cd "${dir}" && bash "${TARGET}" >/dev/null 2>&1)
    code=$?
    set -e
    rm -rf "${dir}"
    if { [ "${expect}" = "pass" ] && [ "${code}" -eq 0 ]; } \
        || { [ "${expect}" = "fail" ] && [ "${code}" -ne 0 ]; }; then
        echo "  ✓ ${name}"
        pass=$((pass + 1))
    else
        echo "  ✗ ${name}（期待 ${expect} / exit ${code}）" >&2
        fail=$((fail + 1))
    fi
}

echo "check-shell-var-brace.sh 回帰テスト"

# 本体: 裸の変数の直後に全角文字。これが #636 の実害そのもの。
check "裸の変数 + 全角括弧" fail "v=abc
echo \"x ${DOLLAR}v）y\""

# ブレースで閉じてあれば安全。
check "ブレース済み + 全角括弧" pass "v=abc
echo \"x ${DOLLAR}{v}）y\""

# 行頭コメントは展開されないので除外する。これにより「この罠を説明するコメント」で
# 悪い例をそのまま書ける（scripts/test-check-adr-numbers.sh がそうしている）。
check "行頭コメント内の悪い例" pass "# ${DOLLAR}label（ と書くと bash が壊れる
echo ok"

# インデント付きの行頭コメントも同じ扱い。
check "インデント付きコメント内の悪い例" pass "if true; then
    # ${DOLLAR}label（ と書くと壊れる
    echo ok
fi"

# 直後が ASCII なら問題ない（過検出しないこと）。
check "直後が ASCII" pass "v=abc
echo \"${DOLLAR}v ok\"
echo \"${DOLLAR}v/path\""

# 位置パラメータは 1 文字で終端するので bash 側も取り込まない。過検出しないこと。
check "位置パラメータ + 全角括弧" pass "echo \"${DOLLAR}1（引数）\""

# 特殊変数も同様。
check "特殊変数 + 全角括弧" pass "false || echo \"rc=${DOLLAR}?（失敗）\""

# 全角に限らず非 ASCII 全般が対象（アクセント付きラテン文字など）。
check "非 ASCII（全角以外）" fail "v=abc
echo \"${DOLLAR}vé\""

# 1 行に複数あるとき、全部拾えること（1 件目で止めない）。
check "1 行に複数" fail "a=1; b=2
echo \"${DOLLAR}a（x）${DOLLAR}b（y）\""

# 行末コメント内は追わない（シェルの字句解析が要る）。検出側に倒す設計であることを固定する。
check "行末コメントは検出側に倒す" fail "echo ok  # ${DOLLAR}v（ここはコメント）"

echo
if [ "${fail}" -ne 0 ]; then
    echo "✗ ${fail} / $((pass + fail)) 件が失敗した" >&2
    exit 1
fi
echo "✓ 全 ${pass} ケース通過"
