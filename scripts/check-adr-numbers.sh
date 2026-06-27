#!/usr/bin/env bash
# ADR 番号（docs/adr/NNNN-kebab-title.md の先頭 4 桁）の重複を機械的に検出する（#254）。
#
# ADR はファイル名でローカル採番するため、並行クローン/worktree 運用では番号が二重取得
# されうる（実例: #251 と #253 が同時に 0040 を採番）。GitHub Issue 番号と違いサーバ採番では
# ないため人手では再発が防げない。本スクリプトを CI / pre-push で走らせて重複を弾く。
#
# 検出タイミングの限界（重要）: pull_request CI はマージ ref 内のスナップショットしか見ない。
# 別々の PR が各々 0040 を採番した場合、各 PR 単体では 0040 が 1 件なので CI は緑で通り、両者
# マージ後の main push CI で初めて落ちる（=事後検出）。PR 段階で確実に弾くには branch protection
# の "Require branches to be up to date before merging" を有効化し、本ジョブを required にする
# （先行 PR マージ後に後続 PR の CI が新 base で再実行され、その時点で重複を検出できる）。
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
if ! repo_root="$(git rev-parse --show-toplevel 2>/dev/null)"; then
    echo "git リポジトリ外では実行できない（docs/adr の解決にリポジトリルートが必要）" >&2
    exit 1
fi
adr_dir="$repo_root/docs/adr"

if [[ ! -d "$adr_dir" ]]; then
    echo "ADR ディレクトリが見つからない: $adr_dir" >&2
    exit 1
fi

# docs/adr 直下の *.md を走査する。番号抽出は「先頭 4 桁」という緩いパターンで行い、
# kebab 規約（NNNN-kebab-title.md）への適合は別軸の警告として扱う。番号抽出を規約適合と
# 同じ厳格パターンに縛ると、規約外ファイル（例: 0040-Foo.md, 0040_foo.md）が重複していても
# 検出網から漏れ、コア保証（番号の重複を必ず弾く）が崩れるため。
declare -a numbers=()        # 重複検出・next 算出に使う 4 桁番号（緩いパターンで抽出）
declare -a nonconforming=()  # kebab 規約に外れるファイル名（警告のみ）
shopt -s nullglob
for path in "$adr_dir"/*.md; do
    base="$(basename "$path")"
    # 既知の非 ADR ファイルは対象外（将来 README/テンプレを置いても警告ノイズを出さない）。
    case "$base" in
        README.md | template.md | TEMPLATE.md) continue ;;
    esac
    if [[ "$base" =~ ^([0-9]{4}) ]]; then
        numbers+=("${BASH_REMATCH[1]}")
        # 番号は取れるが kebab 規約に外れるものは警告対象にする（重複検出からは漏らさない）。
        if [[ ! "$base" =~ ^[0-9]{4}-[a-z0-9]+(-[a-z0-9]+)*\.md$ ]]; then
            nonconforming+=("$base")
        fi
    else
        # 先頭 4 桁すら無いファイルは ADR ではない疑い。気づけるよう警告に載せる。
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
        for path in "$adr_dir/$num"*.md; do
            [[ -e "$path" ]] && echo "    $(basename "$path")" >&2
        done
    done <<<"$dups"
    echo "" >&2
    echo "採番を振り直す: 次に使うべき番号は $(compute_next)（scripts/check-adr-numbers.sh next）" >&2
    exit 1
fi

echo "✓ ADR 番号に重複なし（${#numbers[@]} 件）。次に使うべき番号: $(compute_next)"
