#!/usr/bin/env bash
# check-adr-numbers.sh の回帰テスト（ADR 0073）。
#
# 同スクリプトは ADR 0073 で fail-closed 化した（0 埋め忘れの ADR / ADR 0 件 / 旧 docs/adr の
# 復活を致命扱いにする）。fail-closed の判定は「壊れたときに黙って緑になる」形で退化しやすく、
# 退化しても本番の docs/original-docs は正常なので気づけない。使い捨ての fixture リポジトリを
# 作って各分岐の終了コードを固定する。
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

# ADR らしい本文を書く。判定は H1 の書式ではなく「## ステータス」と「## 決定」の同時存在で
# 行うため、テスト側もその構造を再現する。
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

# 期待する終了コードと実際を突き合わせる。
expect_exit() {
    local label="$1" expected="$2"
    shift 2
    local actual=0
    ( cd "$work/repo" && "$@" >/dev/null 2>&1 ) || actual=$?
    if [[ "$actual" -eq "$expected" ]]; then
        # 変数の直後に全角文字が続く箇所は必ずブレースで閉じる。$label（ と書くと bash が
        # 全角括弧の UTF-8 バイトまで識別子の一部として読み、unbound variable で落ちる。
        echo "  ✓ ${label}（exit ${actual}）"
    else
        echo "  ✗ $label: 期待 exit $expected / 実際 exit $actual" >&2
        failures=$((failures + 1))
    fi
}

# 標準出力に期待する文字列が含まれるかを見る。
expect_stdout_contains() {
    local label="$1" needle="$2"
    shift 2
    local out
    out="$( cd "$work/repo" && "$@" 2>/dev/null )" || true
    if [[ "$out" == *"$needle"* ]]; then
        echo "  ✓ $label"
    else
        echo "  ✗ $label: 出力に '$needle' が含まれない（実際: '$out'）" >&2
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
expect_stdout_contains "next が最大+1 を返す" "0011" bash scripts/check-adr-numbers.sh next

echo "[2] 一次資料・README が混在しても誤検知しない"
reset_fixture
write_adr "$work/repo/docs/original-docs/0001-first.md" "0001. 最初の決定"
printf '# original-docs — 一次資料\n' >"$work/repo/docs/original-docs/README.md"
printf '# 原資料: #382 ライブ EV\n\n生ログ。\n' >"$work/repo/docs/original-docs/382-live.md"
printf '# 一次資料: #401 部分一致\n\n調査ノート。\n' >"$work/repo/docs/original-docs/401-partial.md"
expect_exit "issue 由来の一次資料は黙ってスキップされる" 0 bash scripts/check-adr-numbers.sh

echo "[3] 番号重複を検出する"
reset_fixture
write_adr "$work/repo/docs/original-docs/0040-a.md" "0040. A"
write_adr "$work/repo/docs/original-docs/0040-b.md" "0040. B"
expect_exit "重複で落ちる" 1 bash scripts/check-adr-numbers.sh

echo "[4] 0 埋めを忘れた ADR を致命で拾う（fail-closed）"
reset_fixture
write_adr "$work/repo/docs/original-docs/0001-first.md" "0001. 最初の決定"
write_adr "$work/repo/docs/original-docs/71-forgot-padding.md" "0071. 0 埋め忘れ"
expect_exit "check が落ちる" 1 bash scripts/check-adr-numbers.sh
expect_exit "next も落ちる（採番経路にも効く）" 1 bash scripts/check-adr-numbers.sh next

echo "[5] 2 桁 ADR も拾う（H1 の桁数に依存しない構造判定）"
reset_fixture
write_adr "$work/repo/docs/original-docs/0001-first.md" "0001. 最初の決定"
write_adr "$work/repo/docs/original-docs/74-two-digit.md" "74. 2 桁番号"
expect_exit "check が落ちる" 1 bash scripts/check-adr-numbers.sh

echo "[6] ADR 0 件で落ちる（旧実装は exit 0 の fail-open だった）"
reset_fixture
printf '# 原資料: #382 ライブ EV\n\n生ログ。\n' >"$work/repo/docs/original-docs/382-live.md"
expect_exit "check が落ちる" 1 bash scripts/check-adr-numbers.sh
expect_exit "next も落ちる" 1 bash scripts/check-adr-numbers.sh next

echo "[7] 廃止済み docs/adr の復活を検出する"
reset_fixture
write_adr "$work/repo/docs/original-docs/0001-first.md" "0001. 最初の決定"
mkdir -p "$work/repo/docs/adr"
write_adr "$work/repo/docs/adr/0072-stray.md" "0072. 別 PR が旧パスに足した ADR"
expect_exit "check が落ちる" 1 bash scripts/check-adr-numbers.sh

echo "[8] 引数処理"
reset_fixture
write_adr "$work/repo/docs/original-docs/0001-first.md" "0001. 最初の決定"
expect_exit "-h は exit 0" 0 bash scripts/check-adr-numbers.sh -h
expect_exit "不明な引数は exit 2" 2 bash scripts/check-adr-numbers.sh bogus
expect_exit "引数が多すぎる場合も exit 2" 2 bash scripts/check-adr-numbers.sh check next

echo ""
if [[ "$failures" -gt 0 ]]; then
    echo "✗ $failures 件のケースが失敗した" >&2
    exit 1
fi
echo "✓ 全ケース通過"
