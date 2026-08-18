#!/usr/bin/env bash
# scripts/check-shell-var-nonascii.sh の回帰テスト（#636）。
#
# **なぜ検査にテストが要るか**: リポジトリは常に合格側なので、検査が壊れても本番データが正常だと
# 無言で緑になる。ADR 0073 が `adr` ジョブを「回帰テスト → 本番検査」の順にしたのと同じ理由で、
# fail-closed を謳う検査ほど自身の回帰を持つ必要がある。
#
# 一時 git リポジトリにファイルを置いて検査を走らせる（検査は git ls-files で対象を列挙するので
# git init と git add が要る）。**実行時の bash の挙動には依存させない**——本件はロケール依存で
# プラットフォームごとに再現性が変わるため、検査も回帰も字面の静的判定だけで完結させる。
#
# **フィクスチャの `$` は変数経由で組み立てる**（下の DOLLAR）。悪い例をリテラルで書くと、
# このファイル自身が検査に引っかかる。検査側にテストファイルの除外を入れると
# 「検査が自分自身に適用されない」穴になるので、書く側で回避する。
#
# **期待 exit code と検出件数の両方を突き合わせる。** exit code だけだと「1 件目で止める」退行や
# 内部エラーによる非 0 を見逃す（どちらも fail 期待ケースが緑のまま通ってしまう）。
set -euo pipefail

TARGET=$(cd "$(dirname "$0")" && pwd)/check-shell-var-nonascii.sh
DOLLAR='$'
pass=0
fail=0
WORK=""
# trap の最終コマンドの戻り値がスクリプトの終了コードを上書きするので、必ず 0 で返す
# （`[ -n "" ] && rm` の形だと WORK が空のとき 1 を返し、全ケース通過でも exit 1 になる）。
cleanup() {
    if [ -n "${WORK}" ]; then
        rm -rf "${WORK}"
    fi
    return 0
}
trap cleanup EXIT

# $1 = ケース名 / $2 = 期待 exit（0|1|2）/ $3 = 期待検出件数（exit 1 のときのみ照合。それ以外は -）
# $4.. = "相対パス:本文" の並び
check() {
    local name="$1" want_code="$2" want_hits="$3"
    shift 3
    local code out hits spec path body
    WORK=$(mktemp -d)
    git -C "${WORK}" init -q
    for spec in "$@"; do
        path="${spec%%:*}"
        body="${spec#*:}"
        mkdir -p "${WORK}/$(dirname "${path}")"
        printf '%s\n' "${body}" > "${WORK}/${path}"
        git -C "${WORK}" add -- "${path}"
    done
    set +e
    out=$( (cd "${WORK}" && bash "${TARGET}") 2>&1 )
    code=$?
    set -e
    rm -rf "${WORK}"; WORK=""

    hits=$(printf '%s\n' "${out}" | grep -c '^✗ .*の直後に非 ASCII' || true)
    if [ "${code}" -ne "${want_code}" ]; then
        echo "  ✗ ${name}（期待 exit ${want_code} / 実際 ${code}）" >&2
        fail=$((fail + 1))
        return
    fi
    if [ "${want_hits}" != "-" ] && [ "${hits}" -ne "${want_hits}" ]; then
        echo "  ✗ ${name}（期待 検出 ${want_hits} 件 / 実際 ${hits} 件）" >&2
        fail=$((fail + 1))
        return
    fi
    echo "  ✓ ${name}"
    pass=$((pass + 1))
}

echo "check-shell-var-nonascii.sh 回帰テスト"

# 本体: 裸の変数の直後に全角文字。これが #636 の実害そのもの。
check "裸の変数 + 全角括弧" 1 1 \
    "sample.sh:v=abc
echo \"x ${DOLLAR}v）y\""

# ブレースで閉じてあれば安全。
check "ブレース済み + 全角括弧" 0 - \
    "sample.sh:v=abc
echo \"x ${DOLLAR}{v}）y\""

# 行頭コメントは展開されないので除外する。これにより「この罠を説明するコメント」で
# 悪い例をそのまま書ける（scripts/test-check-adr-numbers.sh がそうしている）。
check "行頭コメント内の悪い例" 0 - \
    "sample.sh:# ${DOLLAR}label（ と書くと bash が壊れる
echo ok"

check "インデント付きコメント内の悪い例" 0 - \
    "sample.sh:if true; then
    # ${DOLLAR}label（ と書くと壊れる
    echo ok
fi"

# 直後が ASCII なら問題ない（過検出しないこと）。
check "直後が ASCII" 0 - \
    "sample.sh:v=abc
echo \"${DOLLAR}v ok\"
echo \"${DOLLAR}v/path\""

# 位置パラメータ・特殊変数は 1 文字で終端するので bash 側も取り込まない。
check "位置パラメータ + 全角括弧" 0 - "sample.sh:echo \"${DOLLAR}1（引数）\""
check "特殊変数 + 全角括弧" 0 - "sample.sh:false || echo \"rc=${DOLLAR}?（失敗）\""

# 全角に限らず非 ASCII 全般が対象。
check "非 ASCII（全角以外）" 1 1 \
    "sample.sh:v=abc
echo \"${DOLLAR}vé\""

# **件数まで見る**: 1 件目で止める退行を捕まえる。
check "1 行に複数（2 件とも拾う）" 1 2 \
    "sample.sh:a=1; b=2
echo \"${DOLLAR}a（x）${DOLLAR}b（y）\""

check "複数ファイルにまたがる（3 件とも拾う）" 1 3 \
    "sample.sh:v=1
echo \"${DOLLAR}v（a）\"" \
    "sub/dir/other.sh:w=2
echo \"${DOLLAR}w（b）${DOLLAR}w（c）\""

# 行末コメント内は追わない（字句解析が要る）。検出側に倒す設計であることを固定する。
check "行末コメントは検出側に倒す" 1 1 "sample.sh:echo ok  # ${DOLLAR}v（ここはコメント）"

# **対象列挙の退行を捕まえる**: 拡張子の無い追跡ファイル（scripts/mdq 相当）も対象に含むこと。
check "拡張子なしの対象（scripts/mdq）" 1 1 \
    "scripts/mdq:v=1
echo \"${DOLLAR}v（x）\""

check "拡張子なしの対象（pre-push）" 1 1 \
    "scripts/git-hooks/pre-push:v=1
echo \"${DOLLAR}v（x）\""

# **fail-closed の安全網**: 対象 0 件は「素通り」ではなく内部エラー（exit 2）にする。
check "対象ファイル 0 件は exit 2" 2 - "README.md:not a shell script"

# --- ここから経路別のケース（本体モード以外） ---
# CI の shellcheck ステップは --list の出力を単一ソースとして消費するので、契約（NUL 区切り・
# 件数・fail-closed）を固定する。FILE... モードは cwd がリポジトリルート以外でも動くこと。

path_case() {
    # $1 = ケース名 / $2 = 期待 exit / $3 = 実行する関数名（WORK 内で評価される）
    local name="$1" want_code="$2" body="$3" code
    WORK=$(mktemp -d)
    git -C "${WORK}" init -q
    mkdir -p "${WORK}/sub"
    printf 'v=1\necho "%sv（x）"\n' "${DOLLAR}" > "${WORK}/sub/bad.sh"
    printf 'echo ok\n' > "${WORK}/good.sh"
    git -C "${WORK}" add -- sub/bad.sh good.sh
    set +e
    ( cd "${WORK}" && eval "${body}" ) >/dev/null 2>&1
    code=$?
    set -e
    rm -rf "${WORK}"; WORK=""
    if [ "${code}" -eq "${want_code}" ]; then
        echo "  ✓ ${name}"
        pass=$((pass + 1))
    else
        echo "  ✗ ${name}（期待 exit ${want_code} / 実際 ${code}）" >&2
        fail=$((fail + 1))
    fi
}

# --list: 対象があるときは 0 で NUL 区切り 2 件を返す
path_case "--list は NUL 区切りで列挙する" 0 \
    "test \"\$(bash '${TARGET}' --list | tr -dc '\\0' | wc -c | tr -d ' ')\" -eq 2"
# --list: 対象 0 件は fail-closed（CI が無言で 0 ファイル検査になるのを防ぐ）
path_case "--list も 0 件なら exit 2" 2 \
    "git rm -qf sub/bad.sh good.sh && bash '${TARGET}' --list"
# FILE... モード: **リポジトリルート以外の cwd** から相対パスで呼んでも動く
path_case "FILE... を sub/ から相対パスで渡す" 1 \
    "cd sub && bash '${TARGET}' bad.sh"
path_case "FILE... で違反の無いファイルを渡す" 0 \
    "cd sub && bash '${TARGET}' ../good.sh"
# 追跡済みだが作業ツリーに無いファイルは内部エラーにせずスキップする
path_case "作業ツリーに無い追跡ファイルはスキップ" 0 \
    "rm -f sub/bad.sh && bash '${TARGET}'"

echo
if [ "${fail}" -ne 0 ]; then
    echo "✗ ${fail} / $((pass + fail)) 件が失敗した" >&2
    exit 1
fi
echo "✓ 全 ${pass} ケース通過"
