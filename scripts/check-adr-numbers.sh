#!/usr/bin/env bash
# ADR 番号（docs/adr/NNNN-kebab-title.md の先頭 4 桁）の重複を機械的に検出する（#254）。
#
# ADR はファイル名でローカル採番するため、並行クローン/worktree 運用では番号が二重取得
# されうる（実例: #251 と #253 が同時に 0040 を採番）。GitHub Issue 番号と違いサーバ採番では
# ないため人手では再発が防げない。本スクリプトを CI / pre-push で走らせて重複を弾く。
#
# 使い方:
#   scripts/check-adr-numbers.sh          # 重複検出（重複があれば非ゼロ終了＋該当列挙）
#   scripts/check-adr-numbers.sh check    # 同上
#   scripts/check-adr-numbers.sh next     # 次に使うべき番号（最大+1）を 4 桁で表示
set -euo pipefail

usage() {
    cat <<'EOF'
check-adr-numbers.sh - ADR 番号（docs/adr/NNNN-*.md）の重複検出（#254）

使い方:
  scripts/check-adr-numbers.sh          # 重複検出（重複があれば非ゼロ終了）
  scripts/check-adr-numbers.sh check    # 同上
  scripts/check-adr-numbers.sh next     # 次に使うべき番号（最大+1）を 4 桁で表示

オプション:
  -h, --help   このヘルプ
EOF
}

# どの cwd から呼んでも docs/adr を解決できるようリポジトリルート起点にする。
repo_root="$(git rev-parse --show-toplevel)"
adr_dir="$repo_root/docs/adr"

if [[ ! -d "$adr_dir" ]]; then
    echo "ADR ディレクトリが見つからない: $adr_dir" >&2
    exit 1
fi

# docs/adr 直下の *.md を走査し、規約に合致するもの（NNNN-...）の番号と、
# 合致しないもの（非 ADR）を分けて集める。
declare -a numbers=()
declare -a nonconforming=()
shopt -s nullglob
for path in "$adr_dir"/*.md; do
    base="$(basename "$path")"
    if [[ "$base" =~ ^([0-9]{4})-[a-z0-9]+(-[a-z0-9]+)*\.md$ ]]; then
        numbers+=("${BASH_REMATCH[1]}")
    else
        nonconforming+=("$base")
    fi
done
shopt -u nullglob

# 最大番号+1 を 4 桁で返す（ADR が無ければ 0001）。
compute_next() {
    local max=0 n
    for n in "${numbers[@]:-}"; do
        [[ -z "$n" ]] && continue
        # 10# で 8 進数誤解釈（先頭 0）を防ぐ。
        ((10#$n > max)) && max=$((10#$n))
    done
    printf '%04d\n' $((max + 1))
}

cmd="${1:-check}"
case "$cmd" in
    -h|--help) usage; exit 0 ;;
    next) compute_next; exit 0 ;;
    check) ;;
    *) echo "不明な引数: $cmd" >&2; usage >&2; exit 2 ;;
esac

status=0

# 規約に合致しないファイルは警告のみ（README 等の非 ADR が将来増える可能性を許容）。
if [[ ${#nonconforming[@]} -gt 0 ]]; then
    echo "警告: ADR 命名規約（NNNN-kebab-title.md）に合致しないファイル:" >&2
    printf '  %s\n' "${nonconforming[@]}" >&2
fi

if [[ ${#numbers[@]} -eq 0 ]]; then
    echo "ADR ファイルが見つからない（docs/adr/NNNN-*.md）" >&2
    exit "$status"
fi

# 重複番号を抽出する。出現回数 >= 2 の番号を集める。
dups="$(printf '%s\n' "${numbers[@]}" | sort | uniq -d)"

if [[ -n "$dups" ]]; then
    echo "✗ ADR 番号の重複を検出:" >&2
    while IFS= read -r num; do
        [[ -z "$num" ]] && continue
        echo "  番号 $num:" >&2
        for path in "$adr_dir/$num"-*.md; do
            [[ -e "$path" ]] && echo "    $(basename "$path")" >&2
        done
    done <<<"$dups"
    echo "" >&2
    echo "採番を振り直す: 次に使うべき番号は $(compute_next)（scripts/check-adr-numbers.sh next）" >&2
    exit 1
fi

echo "✓ ADR 番号に重複なし（${#numbers[@]} 件）。次に使うべき番号: $(compute_next)"
