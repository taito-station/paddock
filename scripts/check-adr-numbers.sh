#!/usr/bin/env bash
# ADR 番号（docs/original-docs/0NNN-kebab-title.md の先頭 4 桁）の重複を機械的に検出する（#254）。
#
# ADR はファイル名でローカル採番するため、並行クローン/worktree 運用では番号が二重取得
# されうる（実例: #251 と #253 が同時に 0040 を採番）。GitHub Issue 番号と違いサーバ採番では
# ないため人手では再発が防げない。本スクリプトを CI / pre-push で走らせて重複を弾く。
#
# 走査先は docs/original-docs（ADR 0073 で旧 docs/adr から統合。ADR は一次資料層に属する）。
# このディレクトリには ADR と GitHub issue 由来の一次資料（382-*.md 等）が混在するため、
# ファイル名で両者を分離する。分離規約は docs/original-docs/README.md が正:
#   - ADR     : 0 埋め 4 桁で始まる（0001〜0999）。先頭文字が必ず '0'。
#   - 一次資料 : GitHub issue 番号で始まる。issue 番号は 0 埋めしない（382-, 401- …）。
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
check-adr-numbers.sh - ADR 番号（docs/original-docs/0NNN-*.md）の重複検出（#254）

使い方:
  scripts/check-adr-numbers.sh          # 重複検出（重複があれば非ゼロ終了）
  scripts/check-adr-numbers.sh check    # 同上
  scripts/check-adr-numbers.sh next     # 次に使うべき番号（最大+1）を 4 桁で表示

オプション:
  -h, --help   このヘルプ
EOF
}

# 引数は git 解決・走査より前に検証する（-h/--help と不正引数はリポジトリ外でも応答したい）。
cmd="${1:-check}"
case "$cmd" in
    -h|--help) usage; exit 0 ;;
    next | check) ;;
    *) echo "不明な引数: $cmd" >&2; usage >&2; exit 2 ;;
esac
# 受理するのはサブコマンド 1 つだけ。余分な後続引数は黙殺せず弾く（不明引数と同じ厳格さ）。
if [[ $# -gt 1 ]]; then
    echo "引数が多すぎる（受理は check / next / -h のいずれか 1 つ）: $*" >&2
    usage >&2
    exit 2
fi

# どの cwd から呼んでも docs/original-docs を解決できるようリポジトリルート起点にする。
if ! repo_root="$(git rev-parse --show-toplevel 2>/dev/null)"; then
    echo "git リポジトリ外では実行できない（docs/original-docs の解決にリポジトリルートが必要）" >&2
    exit 1
fi
adr_dir="$repo_root/docs/original-docs"

if [[ ! -d "$adr_dir" ]]; then
    echo "ADR ディレクトリが見つからない: $adr_dir" >&2
    exit 1
fi

# docs/original-docs 直下の *.md を走査する。まず「ADR かどうか」を先頭 '0' で分離し、
# 非 ADR（一次資料・README）は黙ってスキップする（警告に載せるとノイズで本来見るべき
# 重複検出が埋もれる）。ADR と判定したものの番号抽出は「先頭 4 桁」という緩いパターンで
# 行い、kebab 規約（0NNN-kebab-title.md）への適合は別軸の警告として扱う。番号抽出を規約
# 適合と同じ厳格パターンに縛ると、規約外ファイル（例: 0040-Foo.md, 0040_foo.md）が重複
# していても検出網から漏れ、コア保証（番号の重複を必ず弾く）が崩れるため。
declare -a numbers=()        # 重複検出・next 算出に使う 4 桁番号（緩いパターンで抽出）
declare -a nonconforming=()  # ADR だが kebab 規約に外れるファイル名（警告のみ）
declare -a misnamed_adr=()   # ADR に見えるのに 0 埋め 4 桁で始まらない（致命）
shopt -s nullglob
for path in "$adr_dir"/*.md; do
    base="$(basename "$path")"
    if [[ ! "$base" =~ ^0[0-9]{3} ]]; then
        # 非 ADR（一次資料 / README / テンプレ）。黙ってスキップする。
        # ただし 0 埋めを忘れた ADR を取りこぼすと重複検出が静かに無効化される（fail-open）
        # ため、H1 が ADR 書式に見えるものだけは致命として拾う。実在の H1 は 2 系統:
        # 「# ADR 0001: …」と「# 0071. …」。
        if head -n 5 "$path" | grep -qE '^#[[:space:]]+(ADR[[:space:]]+)?[0-9]{3,4}[.:：]'; then
            misnamed_adr+=("$base")
        fi
        continue
    fi
    # 「先頭 4 桁 + 直後が非数字（または終端）」を番号として抽出する。直後を非数字に限定する
    # ことで 5 桁番号（例: 00401-foo.md）を 0040 と誤抽出して偽の重複を出すのを防ぎつつ、
    # ダッシュ無し（0043dup.md）等の規約外も重複検出網には載せる（コア保証を死守）。
    if [[ "$base" =~ ^(0[0-9]{3})([^0-9]|$) ]]; then
        numbers+=("${BASH_REMATCH[1]}")
        # 番号は取れるが kebab 規約に外れるものは警告対象にする（重複検出からは漏らさない）。
        if [[ ! "$base" =~ ^0[0-9]{3}-[a-z0-9]+(-[a-z0-9]+)*\.md$ ]]; then
            nonconforming+=("$base")
        fi
    else
        # 先頭は 0 だが 4 桁境界を満たさない（00401-foo.md 等）。ADR 疑いなので警告に載せる。
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

if [[ "$cmd" == next ]]; then
    compute_next
    exit 0
fi

# 0 埋めを外した ADR は主判定（先頭 '0'）の網から漏れ、重複検出を静かに無効化する。
# 警告では見逃されるので致命扱いにする（fail-closed）。
if [[ ${#misnamed_adr[@]} -gt 0 ]]; then
    echo "✗ ADR に見えるが 0 埋め 4 桁で始まらないファイル（0NNN-*.md にリネームする）:" >&2
    printf '  %s\n' "${misnamed_adr[@]}" >&2
    exit 1
fi

# 規約に合致しないファイルは警告のみ（README 等の非 ADR が将来増える可能性を許容）。
if [[ ${#nonconforming[@]} -gt 0 ]]; then
    echo "警告: ADR 命名規約（0NNN-kebab-title.md）に合致しないファイル:" >&2
    printf '  %s\n' "${nonconforming[@]}" >&2
fi

if [[ ${#numbers[@]} -eq 0 ]]; then
    # ADR が 0 件になることはありえない（71 本が docs/original-docs にある）。0 件＝判定条件か
    # ディレクトリの取り違えなので、静かに緑にせず落とす（旧実装は exit 0 で fail-open だった）。
    echo "ADR ファイルが見つからない（docs/original-docs/0NNN-*.md）。判定条件かディレクトリを確認する" >&2
    exit 1
fi

# 重複番号を抽出する。出現回数 >= 2 の番号を集める。
dups="$(printf '%s\n' "${numbers[@]}" | sort | uniq -d)"

if [[ -n "$dups" ]]; then
    echo "✗ ADR 番号の重複を検出:" >&2
    while IFS= read -r num; do
        [[ -z "$num" ]] && continue
        echo "  番号 $num:" >&2
        for path in "$adr_dir/$num"*.md; do
            [[ -e "$path" ]] || continue
            b="$(basename "$path")"
            # 抽出と同じ境界（4 桁の直後が非数字/終端）で再確認し、5 桁番号ファイル
            # （00401-*.md 等）が $num の重複として誤って列挙されるのを防ぐ。
            [[ "$b" =~ ^${num}([^0-9]|$) ]] && echo "    $b" >&2
        done
    done <<<"$dups"
    echo "" >&2
    echo "採番を振り直す: 次に使うべき番号は $(compute_next)（scripts/check-adr-numbers.sh next）" >&2
    exit 1
fi

echo "✓ ADR 番号に重複なし（${#numbers[@]} 件）。次に使うべき番号: $(compute_next)"
