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
# 受理するのはサブコマンド 1 つだけ。余分な後続引数は黙殺せず弾く（不明引数と同じ厳格さ）。
# 個数チェックは -h の処理より前に置く。後ろに置くと `-h bogus` が黙って成功してしまう。
if [[ $# -gt 1 ]]; then
    echo "引数が多すぎる（受理は check / next / -h のいずれか 1 つ）: $*" >&2
    usage >&2
    exit 2
fi
cmd="${1:-check}"
case "$cmd" in
    -h|--help) usage; exit 0 ;;
    next | check) ;;
    *) echo "不明な引数: $cmd" >&2; usage >&2; exit 2 ;;
esac

# どの cwd から呼んでも docs/original-docs を解決できるようリポジトリルート起点にする。
if ! repo_root="$(git rev-parse --show-toplevel 2>/dev/null)"; then
    echo "git リポジトリ外では実行できない（docs/original-docs の解決にリポジトリルートが必要）" >&2
    exit 1
fi
adr_dir="$repo_root/docs/original-docs"
legacy_adr_dir="$repo_root/docs/adr"

if [[ ! -d "$adr_dir" ]]; then
    echo "ADR ディレクトリが見つからない: $adr_dir" >&2
    exit 1
fi

# コードフェンス（``` / ~~~ で囲まれた範囲）を落とした本文を返す。フェンスの中に貼られた
# ADR 雛形を「本物の見出し」と取り違えないようにするため。
#
# フェンス行が奇数のとき（閉じ忘れ）は **何も落とさず全文を返す**。単純にトグルすると最後の
# フェンス以降が丸ごと消え、そこにある見出しを見落として fail-open になる。判定を緩める側に
# 倒れるくらいなら、誤検知の可能性を受け入れて全文を見る（fail-closed 側に倒す）。
strip_code_fences() {
    awk '
        { lines[NR] = $0 }
        /^[[:space:]]*(```|~~~)/ { fences++ }
        END {
            if (fences % 2 == 1) {
                for (i = 1; i <= NR; i++) { print lines[i] }
                exit
            }
            for (i = 1; i <= NR; i++) {
                if (lines[i] ~ /^[[:space:]]*(```|~~~)/) { in_fence = !in_fence; continue }
                if (!in_fence) { print lines[i] }
            }
        }
    ' "$1"
}

# ファイルが ADR の本文構造（コードフェンスの外の行頭に「## ステータス」と「## 決定」が
# 同時に存在する）を持つか。0 埋めを忘れた ADR を名前以外の手がかりで拾うための判定。
looks_like_adr() {
    local body
    body="$(strip_code_fences "$1")"
    grep -qE '^## ステータス' <<<"$body" && grep -qE '^## 決定' <<<"$body"
}

# 旧 docs/adr が復活していたら落とす。ADR 0073 でこのディレクトリは廃止したが、統合前に
# 分岐した PR が新しい ADR を docs/adr に足すと、パスが異なるため git は競合を報告せず
# 無言でマージされる。復活した ADR は本スクリプトの走査先（docs/original-docs）から見えず、
# 番号重複検出が穴あきのまま静かに通ってしまうため、致命扱いにする。
#
# 判定はディレクトリ存在ではなく **中の *.md の有無** で行う。git は空ディレクトリを追跡
# しないので、空の docs/adr が現れるのは .DS_Store やマージ残骸が居るローカル環境だけ。
# そこで落とすと pre-push が恒久的にブロックされるだけで、防ぎたい事故は何も防げない。
# 直下グロブではなく find で階層ごと見る。直下だけを見ると docs/adr/nested/0072-x.md が
# 素通りし、「復活したら落とす」という保証が名ばかりになる。
declare -a legacy_adrs=()
while IFS= read -r stray; do
    [[ -n "$stray" ]] && legacy_adrs+=("${stray#"$legacy_adr_dir"/}")
done < <(find "$legacy_adr_dir" -name '*.md' -print 2>/dev/null | sort)
if [[ ${#legacy_adrs[@]} -gt 0 ]]; then
    echo "✗ 廃止済みの $legacy_adr_dir に ADR が置かれている（ADR 0073 で docs/original-docs へ統合済み）:" >&2
    printf '    %s\n' "${legacy_adrs[@]}" >&2
    echo "" >&2
    echo "  対処: 上記を docs/original-docs/ へ git mv し、参照元の frontmatter sources と" >&2
    echo "        本文リンクの docs/adr/ を docs/original-docs/ に書き換える" >&2
    exit 1
fi

# サブディレクトリに置かれた ADR も落とす。走査は docs/original-docs 直下限定なので、
# docs/original-docs/adr/0001-x.md のような階層を切られると重複検出・採番の両方から
# 完全に不可視になる（fail-open）。ADR 0073 で「フラットに置く」と決めた以上、階層は事故。
#
# 対象は「0 埋め 4 桁の名前」だけでなく「ADR の本文構造を持つもの」も含める。名前だけで
# 絞ると、0 埋めを忘れた ADR をサブディレクトリに置いたとき（sub/74-foo.md）に、階層ガード
# にも 0 埋め忘れガード（走査が直下限定）にも掛からず完全に不可視になる。
declare -a nested_adrs=()
while IFS= read -r nested; do
    [[ -z "$nested" ]] && continue
    if [[ "$(basename "$nested")" =~ ^0[0-9]{3} ]] || looks_like_adr "$nested"; then
        nested_adrs+=("${nested#"$adr_dir"/}")
    fi
done < <(find "$adr_dir" -mindepth 2 -name '*.md' -print 2>/dev/null | sort)
if [[ ${#nested_adrs[@]} -gt 0 ]]; then
    echo "✗ サブディレクトリに ADR が置かれている（docs/original-docs 直下にフラットに置く）:" >&2
    printf '    %s\n' "${nested_adrs[@]}" >&2
    echo "" >&2
    echo "  対処: docs/original-docs/ 直下へ移す。階層に置かれた ADR は重複検出・採番から見えない" >&2
    exit 1
fi

# docs/original-docs 直下の *.md を走査する。まず「ADR かどうか」をファイル名の先頭 '0' で
# 分離し、非 ADR（一次資料・README）は黙ってスキップする（警告に載せるとノイズで本来見る
# べき重複検出が埋もれる）。番号は「先頭の連続数字」をそのままキーにする。4 桁に切り詰めると
# 5 桁（00401-foo.md）を 0040 と誤抽出して偽の重複を出すし、逆に 4 桁以外を検出網から外すと
# 規約外ファイルの重複が漏れてコア保証（番号の重複を必ず弾く）が崩れるため。
declare -a dup_keys=()       # 重複検出に使う番号キー（先頭の連続数字そのまま。桁数を問わない）
declare -a numbers=()        # next 算出に使う 4 桁番号（規約どおりのものだけ）
declare -a nonconforming=()  # ADR だが kebab 規約に外れるファイル名（警告のみ）
declare -a misnamed_adr=()   # ADR に見えるのに 0 埋め 4 桁で始まらない（致命）
shopt -s nullglob
for path in "$adr_dir"/*.md; do
    base="$(basename "$path")"
    # 既知の非 ADR ファイルは対象外（README / テンプレを置いても誤検知しないように）。
    case "$base" in
        README.md | template.md | TEMPLATE.md) continue ;;
    esac
    if [[ ! "$base" =~ ^0[0-9]{3} ]]; then
        # 非 ADR（issue 由来の一次資料）。黙ってスキップする。
        # ただし 0 埋めを忘れた ADR を取りこぼすと重複検出が静かに無効化される（fail-open）
        # ため、ADR に見えるものだけは致命として拾う。
        #
        # 判定は H1 の書式ではなく **本文の構造** で行う。H1 は「# ADR 0001: …」と
        # 「# 0071. …」の 2 系統に割れているうえ、番号の桁数でマッチさせると (a) 2 桁 ADR
        # （`# 74. …`）を取りこぼし、(b) 一次資料の H1 が `# 401: …` 形式になった途端に
        # 誤検知して CI を全停止させる——fail-closed の誤検知は実害が大きい。
        #
        # 見出しは **行頭に錨を打ち、コードフェンスの中身を除いてから** 見る。original-docs は
        # issue 本文や外部資料の逐語転記を置く層なので、引用（`> ## ステータス`）やコード
        # フェンス（```markdown の中に ADR 雛形を貼る）で見出し文字列に言及することが現実に
        # ある。素の部分一致だとそれだけで致命判定になり、CI と pre-push が全停止する。
        # 実測で現行 ADR 72 本は「フェンス外・行頭」形式を 72/72 満たすため、絞っても
        # 取りこぼしは増えない。
        #
        # 限界（現行コーパスに対する実測であって一般的保証ではない）: 「## ステータス」だけ
        # 書いて「## 決定」が無い下書きや、英語見出し（## Status / ## Decision）の ADR は
        # 素通りする。テンプレートを変えるときはこの判定も併せて見直すこと。
        if looks_like_adr "$path"; then
            misnamed_adr+=("$base")
        fi
        continue
    fi
    # 先頭の連続数字をそのまま番号キーにする（0055-x.md → 0055 / 00401-x.md → 00401）。
    # 桁数の違うもの同士は別キーになるので偽の重複は出ず、同桁の重複は必ず検出できる。
    [[ "$base" =~ ^(0[0-9]+) ]] && dup_keys+=("${BASH_REMATCH[1]}")
    # next の算出には規約どおりの 4 桁だけを使う（5 桁以上を混ぜると採番が壊れる）。
    if [[ "$base" =~ ^(0[0-9]{3})([^0-9]|$) ]]; then
        numbers+=("${BASH_REMATCH[1]}")
        # 番号は取れるが kebab 規約に外れるものは警告対象にする（重複検出からは漏らさない）。
        if [[ ! "$base" =~ ^0[0-9]{3}-[a-z0-9]+(-[a-z0-9]+)*\.md$ ]]; then
            nonconforming+=("$base")
        fi
    else
        # 先頭は 0 だが 4 桁境界を満たさない（00401-foo.md 等）。ADR 疑いなので警告に載せる
        # （dup_keys には載っているので重複検出網からは漏れない）。
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
    # 0 埋め 4 桁（0001〜0999）の上限に達したら番号を配らない。1000 を返しても、その名前で
    # ファイルを作った瞬間に「ADR に見えるが 0 埋め 4 桁で始まらない」で恒久的に落ちる
    # ＝使えない番号を配ることになる。規約と判定を併せて見直すのが正しい対処。
    if [[ $max -ge 999 ]]; then
        echo "✗ ADR 番号が上限 0999 に達した（現在の最大: $(printf '%04d' "$max")）" >&2
        echo "  桁数の規約（docs/original-docs/README.md）と本スクリプトの判定を併せて見直す" >&2
        exit 1
    fi
    printf '%04d\n' $((max + 1))
}

# --- 致命チェックは next / check の両方に効かせる（ここより前に置く）---
# next は「次に使うべき番号」を配る経路なので、check だけを fail-closed にしても不十分。
# 走査が壊れている状態（0 埋め忘れの ADR が網から漏れている・ADR 0 件）で next が番号を
# 返すと、既存 ADR と衝突する採番をそのまま配ってしまう。

# 0 埋めを外した ADR は主判定（先頭 '0'）の網から漏れ、重複検出を静かに無効化する。
# 警告では見逃されるので致命扱いにする（fail-closed）。
if [[ ${#misnamed_adr[@]} -gt 0 ]]; then
    echo "✗ ADR に見えるが 0 埋め 4 桁で始まらないファイル（0NNN-*.md にリネームする）:" >&2
    printf '  %s\n' "${misnamed_adr[@]}" >&2
    exit 1
fi

if [[ ${#numbers[@]} -eq 0 ]]; then
    # ADR が 0 件になることはありえない（docs/original-docs に 72 本ある）。0 件＝判定条件か
    # ディレクトリの取り違えなので、静かに緑にせず落とす（旧実装は exit 0 で fail-open だった）。
    echo "ADR ファイルが見つからない（docs/original-docs/0NNN-*.md）。判定条件かディレクトリを確認する" >&2
    exit 1
fi

if [[ "$cmd" == next ]]; then
    compute_next
    exit 0
fi

# 規約に合致しないファイルは警告のみ（命名の揺れは重複検出を壊さないため）。
if [[ ${#nonconforming[@]} -gt 0 ]]; then
    echo "警告: ADR 命名規約（0NNN-kebab-title.md）に合致しないファイル:" >&2
    printf '  %s\n' "${nonconforming[@]}" >&2
fi

# 重複番号を抽出する。出現回数 >= 2 の番号を集める。規約外の桁数（00401-*.md 等）も
# dup_keys に載っているので、コア保証（番号の重複を必ず弾く）は命名の揺れに影響されない。
dups="$(printf '%s\n' "${dup_keys[@]}" | sort | uniq -d)"

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
