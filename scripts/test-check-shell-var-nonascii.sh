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

# **git の環境変数を必ず捨てる**（#645）。これを残したまま git hook から呼ばれると、下で
# `git init` した一時リポジトリの中で走る git が**環境変数側を優先して別のリポジトリを指す**。
# 結果 (a) `git ls-files` が本物のファイルを返し「対象 0 件」系のケースが落ちる、
# (b) 一時リポジトリのつもりの `git add` が**本物の index を汚染する**（フィクスチャが
# ステージされ、未コミットの変更を持つ人の push を巻き込む）。
#
# **どの変数が来るかは実測済み**（git 2.53.0）: 通常リポジトリからの push では GIT_* は渡らず、
# **linked worktree から push したときだけ `GIT_DIR` が入る**（`GIT_WORK_TREE` は入らない）。
# このプロジェクトが issue ごとに worktree を切る運用（`.claude/worktrees/`）なので全員が踏む。
# `GIT_INDEX_FILE` は commit 系フックが渡す変数で pre-push では来ないが、単体で与えるだけで
# 上記 (b) の汚染が再現するため同時に落とす。
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE

# **再帰ブレーキ**（#645）。下の自己回帰は自分を `--nested` 付きで 1 回だけ呼ぶが、その判定を
# 壊すと**赤いテストではなく無限再帰**になる（実測: 90 秒でプロセス 24 個、自力終了せず
# `mktemp -d` を撒き続ける）。pre-push 上では「CPU を焼き続けるハングした push」になり、
# 落ちてくれた方がまだ良い。深さを第 2 引数で受けて 1 段を超えたら即座に落とす。
if [ "${2:-0}" -ge 2 ]; then
    echo "✗ 自己回帰が再帰した（深さ ${2}）。--nested の判定が壊れていないか（#645）" >&2
    exit 1
fi
# 深さ 1（＝自己回帰の子）は自己回帰を回さない。`--nested` の判定だけに頼ると、そこを壊した
# ときに落ちずに無限再帰するので、深さでも二重に止める。
if [ "${2:-0}" -ge 1 ]; then
    set -- --nested "${2:-0}"
fi

TARGET=$(cd "$(dirname "$0")" && pwd)/check-shell-var-nonascii.sh
DOLLAR='$'
pass=0
fail=0
WORK=""
# 自己回帰が作る使い捨てリポジトリ（#645）。`set -e` で途中中断すると本体側の `rm -rf` に
# 到達せず /tmp に残るので、こちらも trap で消す（実測: init や staged 計測を失敗させると
# 1 件残っていた）。
SCRATCH=""
# trap の最終コマンドの戻り値がスクリプトの終了コードを上書きするので、必ず 0 で返す
# （`[ -n "" ] && rm` の形だと WORK が空のとき 1 を返し、全ケース通過でも exit 1 になる）。
cleanup() {
    if [ -n "${WORK}" ]; then
        rm -rf "${WORK}"
    fi
    if [ -n "${SCRATCH}" ]; then
        rm -rf "${SCRATCH}"
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

# 明示引数の欠損・未知フラグは fail-open にしない（列挙経路の warn+skip とは分ける）
path_case "FILE... に存在しないパスは exit 2" 2 \
    "bash '${TARGET}' nosuch.sh"
path_case "未知のオプションは exit 2" 2 \
    "bash '${TARGET}' --help"
path_case "--list に余計な引数は exit 2" 2 \
    "bash '${TARGET}' --list good.sh"
# --list は位置を問わず単独指定のときだけ有効（`FILE --list` が黙って FILE モードで走らない）
path_case "--list を後置しても exit 2" 2 \
    "bash '${TARGET}' good.sh --list"
# デコード不能バイトは surrogateescape で読むので「検査不能」ではなく検出側に倒れる
path_case "非 UTF-8 バイトでも落ちずに検出する" 1 \
    "printf 'v=1\\necho \"%sv\\xff\"\\n' '${DOLLAR}' > sub/bin.sh && git add sub/bin.sh && bash '${TARGET}'"

# **実リポジトリ**に対する縮退ガード。`git ls-files` はリテラルパスが一致しなくても rc=0 で
# 黙って無視するので、`scripts/mdq` などを改名すると検査と shellcheck が無言で対象を失う。
# フィクスチャではなく本物のリポジトリで固定する（フィクスチャ側に置くと全ケースが
# この 2 本の配置を強要される）。
repo_root=$(cd "$(dirname "$0")/.." && pwd)
for literal in scripts/mdq scripts/git-hooks/pre-push; do
    if git -C "${repo_root}" ls-files --error-unmatch -- "${literal}" >/dev/null 2>&1; then
        echo "  ✓ リテラル対象が追跡されている（${literal}）"
        pass=$((pass + 1))
    else
        echo "  ✗ リテラル対象が追跡されていない: ${literal}" >&2
        echo "    改名・移動したなら check-shell-var-nonascii.sh の LITERAL_TARGETS も直すこと" >&2
        fail=$((fail + 1))
    fi
done

# **自己回帰（#645）**: git hook 相当の環境で自分をもう 1 回走らせ、同じ結果になることを確かめる。
#
# 単発のケースでは守れない: 壊れ方が「一時リポジトリ内の git が環境変数側を優先する」なので、
# 一時リポジトリを作る**全ケースが同時に**影響を受ける。まるごと再実行が唯一の忠実な再現。
#
# **子プロセスに渡す GIT_* は使い捨ての scratch repo に向ける。** 実リポジトリに向けると、
# 退行が入ったときに**検出行為そのものが本物の index を汚す**（#645 の実害を自分で再現して
# しまう）。これが scratch を使う第一の理由。
#
# ついでに「実行後に scratch の index が空か」も見るが、**これは多重防御であって単独では
# load-bearing ではない**——現行の退行（unset の要素落ち）では子が先に非 0 で落ちるため、
# 汚染判定に到達する前に exit code 側が捕まえる。`staged` の判定を潰す変異は緑のまま生存する。
# 「子が成功したのに汚す」経路が将来生まれたときの保険として残す。
#
# 2 条件を張る:
#   - `git_dir_only`: **実際の pre-push が渡す形**（linked worktree からの push でだけ GIT_DIR が入る。
#     通常リポジトリからの push では GIT_* は渡らない）
#   - `full_env`: unset リストの全要素（GIT_WORK_TREE / GIT_INDEX_FILE も落としていることを固定）
#
# **この 2 条件が別物であることを機械は見ていない。** 件数チェックは「何件走ったか」しか数えない
# ので、両方を `git_dir_only` に複製したうえで `unset` から GIT_INDEX_FILE を外す、という
# 2 箇所同時の改変は緑のまま通る（実測）。条件を減らす／`unset` を単独で崩す変異は捕まる。
#
# ネストは**環境変数ではなく引数**で止める。env で切れる作りだと `export` 一発で回帰が無言で
# 消える（実行時間を惜しむ人が必ずやる）。実行コストは約 7 秒 ×2 で、pre-push 全体（clippy
# 十数分）に対して無視できる。
if [ "${1:-}" != "--nested" ]; then
    for cond in git_dir_only full_env; do
        SCRATCH=$(mktemp -d)
        git -C "${SCRATCH}" init -q
        case "${cond}" in
        git_dir_only) env_args=(GIT_DIR="${SCRATCH}/.git") ;;
        *) env_args=(
            GIT_DIR="${SCRATCH}/.git"
            GIT_WORK_TREE="${SCRATCH}"
            GIT_INDEX_FILE="${SCRATCH}/.git/index"
        ) ;;
        esac
        # bash 3.2 は `set -u` 下で空配列の展開が unbound エラーになる。今は case の
        # 両腕が必ず 1 要素以上を入れるので安全だが、env 無しの条件（plain）を足すと
        # **テストが赤くなるのではなくスクリプトごと落ちる**ので、そのときは
        # `${env_args[@]+"${env_args[@]}"}` に変えること。
        code=0
        # 深さは**受け取った値 +1**で渡す。ここを定数にすると深さが増えず再帰ブレーキが効かない
        # （実際に定数 1 で書いて M5 の無限再帰を止められなかった）。
        out=$(env "${env_args[@]}" bash "$0" --nested "$((${2:-0} + 1))" 2>&1) || code=$?
        # **汚染は exit code と独立に測る。** 成功した経路には汚染が起こり得ないので、
        # ここを `if` の成功側だけに置くと「起こり得ない場所だけを見張る」assert になる
        # （実際、unset から GIT_INDEX_FILE を外す退行では失敗側で staged=8 になっていた）。
        staged=$(git -C "${SCRATCH}" diff --cached --name-only | wc -l | tr -d ' ')
        rm -rf "${SCRATCH}"
        SCRATCH=""

        if [ "${code}" -ne 0 ]; then
            echo "  ✗ git hook 相当の環境で失敗した（${cond}）" >&2
            echo "    冒頭の unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE が外れていないか（#645）" >&2
            printf '%s\n' "${out}" | sed 's/^/    | /' >&2
            fail=$((fail + 1))
        elif [ "${staged}" -ne 0 ]; then
            echo "  ✗ git hook 相当の環境で外部リポジトリの index を汚した（${cond}: ${staged} 件）" >&2
            echo "    冒頭の unset が外れていないか（#645 の実害 2）" >&2
            fail=$((fail + 1))
        else
            echo "  ✓ git hook 相当の環境でも通り外部リポジトリを汚さない（${cond}）"
            pass=$((pass + 1))
        fi
    done
fi

# **ケース数の完全一致**（#645）。ケースが無言で消えても「全 N ケース通過」で緑になるのを防ぐ。
# ケースを増減したら `BASE_CASES` を直す（fail-closed。ADR 0073 と同じ方針）。
#
# **`-lt` ではなく `-ne`。** 下限だけだと「ケースを足して下限を上げ忘れる」が無警告で通り、
# その状態で別のケースが消えると**現行と 1 文字も違わない出力**で緑になる（実測）。
# 完全一致ならケース追加が即座に赤くなり、同期が強制される。
#
# **数値は 1 つだけ持つ。** nested / 非 nested で別々の定数を置くと「片方だけ直す」事故が起きる。
#
# **自己回帰の if の外に置く。** 中に入れると、上のガードを 1 か所いじるだけで自己回帰と件数
# チェックが同時に消え、`✓ 全 26 ケース通過` で緑になる（実際にその変異が生存した）。
BASE_CASES=26
if [ "${1:-}" = "--nested" ]; then
    want_cases=${BASE_CASES}
else
    want_cases=$((BASE_CASES + 2)) # 自己回帰の 2 条件ぶん
fi
if [ "$((pass + fail))" -ne "${want_cases}" ]; then
    echo "✗ ケース数が ${want_cases} と一致しない（$((pass + fail)) 件）" >&2
    echo "  ケースを増減したなら BASE_CASES を直すこと。無言で消えていないかも疑う" >&2
    exit 1
fi

echo
if [ "${fail}" -ne 0 ]; then
    echo "✗ ${fail} / $((pass + fail)) 件が失敗した" >&2
    exit 1
fi
echo "✓ 全 ${pass} ケース通過"
