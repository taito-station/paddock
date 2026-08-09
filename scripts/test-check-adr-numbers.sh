#!/usr/bin/env bash
# check-adr-numbers.sh の回帰テスト（ADR 0073）。
#
# 同スクリプトは ADR 0073 で fail-closed 化した（0 埋め忘れの ADR / ADR 0 件 / 旧 docs/adr の
# 復活 / サブディレクトリ配置を致命扱いにする）。fail-closed の判定は「壊れたときに黙って緑に
# なる」形で退化しやすく、退化しても本番の docs/original-docs は正常なので気づけない。使い捨ての
# fixture リポジトリを作って各分岐の終了コードと出力を固定する。
#
# 終了コード 1 は複数の分岐で共通なので、致命系は stderr の文言まで突き合わせて
# 「意図した理由で落ちたか」を確認する（別の理由で落ちてもテストが緑になるのを防ぐ）。
#
# 一時ディレクトリ内で完結し、リポジトリ本体には一切触れない。
#
# 使い方:
#   scripts/test-check-adr-numbers.sh
set -euo pipefail

usage() {
    cat <<'EOF'
test-check-adr-numbers.sh - check-adr-numbers.sh の回帰テスト（ADR 0073）

使い方:
  scripts/test-check-adr-numbers.sh    # 全ケースを実行（1 件でも失敗すれば非ゼロ終了）

オプション:
  -h, --help   このヘルプ
EOF
}

case "${1:-}" in
    -h | --help)
        usage
        exit 0
        ;;
    "") ;;
    *)
        echo "不明な引数: $1" >&2
        usage >&2
        exit 2
        ;;
esac

if ! repo_root="$(git rev-parse --show-toplevel 2>/dev/null)"; then
    echo "git リポジトリ外では実行できない" >&2
    exit 1
fi
target="$repo_root/scripts/check-adr-numbers.sh"
if [[ ! -f "$target" ]]; then
    echo "テスト対象が見つからない: $target" >&2
    exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

failures=0

# fixture リポジトリを作り直す。check-adr-numbers.sh は git rev-parse --show-toplevel で
# ルートを解決するため、fixture 側も git リポジトリである必要がある。
reset_fixture() {
    rm -rf "$work/repo"
    mkdir -p "$work/repo/scripts" "$work/repo/docs/original-docs"
    git -C "$work/repo" init -q
    cp "$target" "$work/repo/scripts/check-adr-numbers.sh"
}

# ADR らしい本文を書く。判定は H1 の書式ではなく「## ステータス」と「## 決定」が
# **行頭に** 同時存在するかで行うため、テスト側もその構造を再現する。
write_adr() {
    local path="$1" title="$2"
    cat >"$path" <<EOF
# $title

## ステータス

承認済み。

## コンテキスト

テスト用。

## 決定

テスト用。
EOF
}

# 直近の実行結果を保持する（失敗時に原因を出せるようにする）。
last_status=0
last_output=""
run_target() {
    last_output="$( cd "$work/repo" && "$@" 2>&1 )" && last_status=0 || last_status=$?
}

# 期待する終了コードと実際を突き合わせる。失敗時は実出力も出す（CI で原因が分かるように）。
expect_exit() {
    local label="$1" expected="$2"
    shift 2
    run_target "$@"
    if [[ "$last_status" -eq "$expected" ]]; then
        # 変数の直後に全角文字が続く箇所は必ずブレースで閉じる。$label（ と書くと bash が
        # 全角括弧の UTF-8 バイトまで識別子の一部として読み、unbound variable で落ちる。
        echo "  ✓ ${label}（exit ${last_status}）"
    else
        echo "  ✗ ${label}: 期待 exit ${expected} / 実際 exit ${last_status}" >&2
        echo "    --- 実出力 ---" >&2
        echo "$last_output" | sed 's/^/    /' >&2
        failures=$((failures + 1))
    fi
}

# 終了コードに加えて出力の文言も固定する。exit 1 は複数の分岐で共通なので、
# 「意図した理由で落ちたか」まで見ないと別原因の失敗を取り違える。
expect_exit_with() {
    local label="$1" expected="$2" needle="$3"
    shift 3
    run_target "$@"
    if [[ "$last_status" -eq "$expected" && "$last_output" == *"$needle"* ]]; then
        echo "  ✓ ${label}（exit ${last_status}）"
    else
        echo "  ✗ ${label}: 期待 exit ${expected} かつ出力に '${needle}'" >&2
        echo "    実際 exit ${last_status}" >&2
        echo "    --- 実出力 ---" >&2
        echo "$last_output" | sed 's/^/    /' >&2
        failures=$((failures + 1))
    fi
}

echo "check-adr-numbers.sh 回帰テスト"

echo "[1] 正常系（ADR 3 本・重複なし）"
reset_fixture
write_adr "$work/repo/docs/original-docs/0001-first.md" "0001. 最初の決定"
write_adr "$work/repo/docs/original-docs/0002-second.md" "ADR 0002: 次の決定"
write_adr "$work/repo/docs/original-docs/0010-third.md" "0010. 三つ目"
expect_exit "check が通る" 0 bash scripts/check-adr-numbers.sh
expect_exit_with "next が最大+1 を返す" 0 "0011" bash scripts/check-adr-numbers.sh next

echo "[2] 一次資料・README が混在しても誤検知しない"
reset_fixture
write_adr "$work/repo/docs/original-docs/0001-first.md" "0001. 最初の決定"
printf '# original-docs — 一次資料\n' >"$work/repo/docs/original-docs/README.md"
printf '# 原資料: #382 ライブ EV\n\n生ログ。\n' >"$work/repo/docs/original-docs/382-live.md"
printf '# 一次資料: #401 部分一致\n\n調査ノート。\n' >"$work/repo/docs/original-docs/401-partial.md"
expect_exit "issue 由来の一次資料は黙ってスキップされる" 0 bash scripts/check-adr-numbers.sh

echo "[3] 一次資料が本文中で ADR 見出しに言及しても誤検知しない（行頭アンカー）"
reset_fixture
write_adr "$work/repo/docs/original-docs/0001-first.md" "0001. 最初の決定"
# 引用・コードフェンス内の言及。行頭アンカーが無いと致命判定になり CI が全停止する。
{
    printf '# 原資料: #384 調査\n\n'
    printf '> ADR 0055 の「## ステータス」節を見よ。判断は「## 決定」節にある。\n\n'
    printf '```markdown\n## ステータス\n承認済み\n## 決定\nこうする\n```\n'
} >"$work/repo/docs/original-docs/384-quote.md"
expect_exit "引用・コードフェンス内の見出し言及は無視される" 0 bash scripts/check-adr-numbers.sh

echo "[4] 番号重複を検出する"
reset_fixture
write_adr "$work/repo/docs/original-docs/0040-a.md" "0040. A"
write_adr "$work/repo/docs/original-docs/0040-b.md" "0040. B"
expect_exit_with "重複で落ちる" 1 "ADR 番号の重複を検出" bash scripts/check-adr-numbers.sh

echo "[5] 規約外の桁数でも重複検出網から漏れない（コア保証）"
reset_fixture
write_adr "$work/repo/docs/original-docs/0001-first.md" "0001. 最初の決定"
write_adr "$work/repo/docs/original-docs/00401-a.md" "00401. A"
write_adr "$work/repo/docs/original-docs/00401-b.md" "00401. B"
expect_exit_with "5 桁の同番号を重複として検出する" 1 "番号 00401" bash scripts/check-adr-numbers.sh

echo "[6] 規約外の命名は警告のみで落とさない"
reset_fixture
write_adr "$work/repo/docs/original-docs/0001-first.md" "0001. 最初の決定"
write_adr "$work/repo/docs/original-docs/0042-Foo_Bar.md" "0042. 大文字とアンダースコア"
expect_exit_with "kebab 規約違反は警告どまり" 0 "ADR 命名規約" bash scripts/check-adr-numbers.sh
expect_exit_with "4 桁の重複判定は壊れない（0043 は別番号）" 0 "重複なし" bash scripts/check-adr-numbers.sh

echo "[7] 0 埋めを忘れた ADR を致命で拾う（fail-closed）"
reset_fixture
write_adr "$work/repo/docs/original-docs/0001-first.md" "0001. 最初の決定"
write_adr "$work/repo/docs/original-docs/71-forgot-padding.md" "0071. 0 埋め忘れ"
expect_exit_with "check が落ちる" 1 "0 埋め 4 桁で始まらない" bash scripts/check-adr-numbers.sh
expect_exit_with "next も落ちる（採番経路にも効く）" 1 "0 埋め 4 桁で始まらない" bash scripts/check-adr-numbers.sh next

echo "[8] 2 桁 ADR も拾う（H1 の桁数に依存しない構造判定）"
reset_fixture
write_adr "$work/repo/docs/original-docs/0001-first.md" "0001. 最初の決定"
write_adr "$work/repo/docs/original-docs/74-two-digit.md" "74. 2 桁番号"
expect_exit_with "check が落ちる" 1 "0 埋め 4 桁で始まらない" bash scripts/check-adr-numbers.sh

echo "[9] ADR 0 件で落ちる（旧実装は exit 0 の fail-open だった）"
reset_fixture
printf '# 原資料: #382 ライブ EV\n\n生ログ。\n' >"$work/repo/docs/original-docs/382-live.md"
expect_exit_with "check が落ちる" 1 "ADR ファイルが見つからない" bash scripts/check-adr-numbers.sh
expect_exit_with "next も落ちる" 1 "ADR ファイルが見つからない" bash scripts/check-adr-numbers.sh next

echo "[10] 廃止済み docs/adr に ADR が置かれたら落ちる"
reset_fixture
write_adr "$work/repo/docs/original-docs/0001-first.md" "0001. 最初の決定"
mkdir -p "$work/repo/docs/adr"
write_adr "$work/repo/docs/adr/0072-stray.md" "0072. 別 PR が旧パスに足した ADR"
expect_exit_with "check が落ちる" 1 "廃止済み" bash scripts/check-adr-numbers.sh
expect_exit_with "next も落ちる" 1 "廃止済み" bash scripts/check-adr-numbers.sh next

echo "[11] 空の docs/adr では落ちない（ローカル残骸で pre-push を止めない）"
reset_fixture
write_adr "$work/repo/docs/original-docs/0001-first.md" "0001. 最初の決定"
mkdir -p "$work/repo/docs/adr"
touch "$work/repo/docs/adr/.DS_Store"
expect_exit "*.md が無ければ無視する" 0 bash scripts/check-adr-numbers.sh

echo "[12] サブディレクトリの ADR を致命で拾う（走査は直下限定なので不可視になる）"
reset_fixture
write_adr "$work/repo/docs/original-docs/0001-first.md" "0001. 最初の決定"
mkdir -p "$work/repo/docs/original-docs/adr"
write_adr "$work/repo/docs/original-docs/adr/0001-nested-dup.md" "0001. 階層に置かれた重複"
expect_exit_with "check が落ちる" 1 "サブディレクトリ" bash scripts/check-adr-numbers.sh
expect_exit_with "next も落ちる" 1 "サブディレクトリ" bash scripts/check-adr-numbers.sh next

echo "[13] docs/original-docs 自体が無ければ落ちる"
reset_fixture
rm -rf "$work/repo/docs/original-docs"
expect_exit_with "check が落ちる" 1 "ADR ディレクトリが見つからない" bash scripts/check-adr-numbers.sh

echo "[14] 引数処理"
reset_fixture
write_adr "$work/repo/docs/original-docs/0001-first.md" "0001. 最初の決定"
expect_exit "-h は exit 0" 0 bash scripts/check-adr-numbers.sh -h
expect_exit "--help も exit 0" 0 bash scripts/check-adr-numbers.sh --help
expect_exit "不明な引数は exit 2" 2 bash scripts/check-adr-numbers.sh bogus
expect_exit "引数が多すぎる場合は exit 2" 2 bash scripts/check-adr-numbers.sh check next
expect_exit "-h に余分な引数が付いても黙って成功しない" 2 bash scripts/check-adr-numbers.sh -h bogus

echo "[15] git リポジトリ外では落ちる"
rm -rf "$work/nogit"
mkdir -p "$work/nogit"
cp "$target" "$work/nogit/check-adr-numbers.sh"
nogit_status=0
nogit_out="$( cd "$work/nogit" && bash ./check-adr-numbers.sh 2>&1 )" || nogit_status=$?
if [[ "$nogit_status" -eq 1 && "$nogit_out" == *"git リポジトリ外"* ]]; then
    echo "  ✓ git リポジトリ外は exit 1"
else
    echo "  ✗ git リポジトリ外: 期待 exit 1 かつ 'git リポジトリ外' / 実際 exit ${nogit_status}" >&2
    echo "$nogit_out" | sed 's/^/    /' >&2
    failures=$((failures + 1))
fi

echo ""
if [[ "$failures" -gt 0 ]]; then
    echo "✗ ${failures} 件のケースが失敗した" >&2
    exit 1
fi
echo "✓ 全ケース通過"
