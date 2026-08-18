#!/usr/bin/env bash
# シェルスクリプトで `$var` の直後に非 ASCII 文字（全角括弧・読点など）を置いていないか検査する（#636）。
#
# **UTF-8 ロケールの bash は、その非 ASCII のバイトまで変数名に取り込む。** 識別子の終端判定に
# `isalnum()` を使っており、これがロケールのテーブルを見るため。結果 `$pid）` は `pid）` という
# 別名になり、`set -u` 下で `unbound variable` になって落ちる。
#
#   $ LC_ALL=ja_JP.UTF-8 bash -c 'set -u; v=abc; echo "x $v）y"'
#   bash: v?: unbound variable        # LC_ALL=C なら正常に出力される
#
# **バージョンではなくロケールで挙動が変わるのが厄介。** launchd の plist は PATH しか設定しない
# ＝ C ロケールなので常駐ジョブは壊れず、**人が UTF-8 の端末から叩いたときだけ落ちる**。
# 2026-08-16 に deployments/launchd/uninstall.sh がこれで異常終了し、set -u の展開エラーで
# 途中終了したため後続の lock 削除に到達しないという「最後まで走ったように見えて走っていない」
# 状態になった。
#
# **さらに悪いことに、この地雷は失敗報告の行に集中しやすい**（`✗ $name（…）` のような書き方）。
# 何が失敗したかを伝えるはずのメッセージが、まさにその場面で消える。
#
# `shellcheck 0.11.0` はこれを検出しない（--severity=style でも exit 0）ので専用の検査を置く。
# 実行時の挙動はロケールとプラットフォームに依存して確かめにくいため、**静的に字面で禁じる**。
#
# **既知の非カバー範囲**（いずれも現時点で該当 0 件。「検査済み」と誤解しないための記録）:
#   - **展開される複数行文脈の `#` 始まり行**。行頭コメント除外は「その行が本当にコメントか」を
#     見ていないので、**クォート無しヒアドキュメント本文・複数行ダブルクォート文字列の継続行・
#     `$(...)` の中**などが素通りする（いずれも展開されるので実際には落ちる）。追跡には状態機械が
#     要り、検査の単純さと引き換えになるため見送った。**これらの文脈では行頭コメントでも
#     ブレースを付けること**
#   - `.github/workflows/*.yml` の `run:` と `deployments/*.Dockerfile` の `RUN`。これらも
#     UTF-8 ロケールの bash で走るが対象外
#   - **Markdown 内の実行用コードフェンス**（人が端末に貼る運用手順）。`.md` を対象に含めると、
#     この罠を説明する文書が自分の悪い例で引っかかるため入れていない
#
# **「現時点で該当 0 件」は点検時点の観測であって保証ではない。** 上記の非カバー範囲は
# 機械では守られていないので、該当箇所を書くときは人が気をつけること。
#
# **過検出は許容する**（検出側に倒す）: 行末コメント・シングルクォート内・エスケープ済み `\$var` も
# 拾う。いずれも展開されないので無害だが、ブレースを付けて損は無い。
# **ただしシェル以外の言語を埋め込んでいる箇所は例外**——`awk '{print $x（}'` のような埋め込みでは
# `${x}` が別の意味になる。その場合はブレース化ではなくコードの組み替えで回避する。
#
# **この検査自体の回帰テストは scripts/test-check-shell-var-nonascii.sh**（リポジトリは常に合格側
# なので、検査が壊れても本番データが正常だと無言で緑になる。ADR 0073 と同じ理由でテストを持つ）。
#
# 使い方:
#   check-shell-var-nonascii.sh            # 追跡対象の全シェルスクリプトを検査
#   check-shell-var-nonascii.sh --list     # 対象ファイルを NUL 区切りで出力するだけ（CI が消費する）
#   check-shell-var-nonascii.sh FILE...    # 指定ファイルだけ検査
#
# 終了コード: 0=違反なし / 1=違反あり / 2=検査を実行できなかった（内部エラー）
set -euo pipefail

if ! root="$(git rev-parse --show-toplevel 2>/dev/null)"; then
    echo "git リポジトリ外では実行できない（対象ファイルの列挙に git ls-files を使う）" >&2
    exit 2
fi
# FILE... モードの引数は **cd する前に**絶対パス化する。後で解決すると cwd がリポジトリルート以外の
# ときに「読めない」で内部エラーになる（ドキュメント済みの用法が壊れる）。
args=()
for arg in "$@"; do
    case "${arg}" in
        --list)
            # 位置を問わず「単独引数のときだけ」有効にする。第 1 引数のときだけ見る作りだと
            # `... FILE --list` が黙って FILE モードで走る（fail-open）。
            if [ "$#" -ne 1 ]; then
                echo "✗ --list は単独で指定する（他の引数と併用できない）: $*" >&2
                exit 2
            fi
            ;;
        -*)
            echo "✗ 不明なオプション: ${arg}" >&2
            echo "  使い方: check-shell-var-nonascii.sh [--list | FILE...]" >&2
            exit 2
            ;;
        /*) args+=("${arg}") ;;
        *) args+=("${PWD}/${arg}") ;;
    esac
done

cd "${root}"

# 対象ファイルの定義はこの 1 箇所だけに置く。CI の shellcheck ジョブは --list を消費するので、
# pathspec を二重管理しない（ADR 0073「人手の規律に委ねない」）。
# 拡張子で拾えない shebang スクリプトはリテラルで挙げる。**リテラルパスが 1 つも一致しなくても
# `git ls-files` は rc=0 で黙って無視する**ので、改名・移動すると `*.sh` が残るぶん 0 件ガードも
# 発火せず、本検査と shellcheck の両方が無言で対象を失う。この縮退は
# scripts/test-check-shell-var-nonascii.sh が実リポジトリに対して固定する
# （検査側で assert すると、あらゆるフィクスチャにこの 2 本を置く必要が出て回帰テストが歪む）。
LITERAL_TARGETS=(scripts/mdq scripts/git-hooks/pre-push)

list_targets() {
    # -z にするのは core.quotePath（既定 true）が非 ASCII パスを "\346\227\245.sh" 形式へ
    # クォートするのを避けるため。改行を含むパスにも耐える。
    git ls-files -z '*.sh' "${LITERAL_TARGETS[@]}"
}

if [ "${1:-}" = "--list" ]; then
    # 本体側と同じく fail-closed にする。CI は `--list | xargs -0 -r` で消費するので、
    # ここが空を返すと **無言で 0 ファイル検査**になる（xargs -r が no-op になるため）。
    if ! listed=$(list_targets | tr -dc '\0' | wc -c | tr -d ' '); then
        echo "✗ 対象ファイルの列挙に失敗した（git ls-files）" >&2
        exit 2
    fi
    if [ "${listed}" -eq 0 ]; then
        echo "✗ 対象ファイルが 0 件（列挙が壊れている＝検査が素通りする）" >&2
        exit 2
    fi
    list_targets
    exit 0
fi

# 欠損ファイルの扱いを分ける。列挙経路（git ls-files）は未ステージ削除で欠けうるので warn+skip、
# **明示引数の欠損はタイポ・リネーム漏れなので exit 2**（fail-open にしない）。
missing_is_error=0
targets=()
if [ "${#args[@]}" -gt 0 ]; then
    missing_is_error=1
    targets=("${args[@]}")
else
    # mapfile は bash 4+ の組み込みで **macOS の bash 3.2 には無い**（本検査が守ろうとしている
    # 環境そのもの）。読み込みループで代替する。
    while IFS= read -r -d '' target; do
        targets+=("${target}")
    done < <(list_targets)
fi

if [ "${#targets[@]}" -eq 0 ]; then
    echo "✗ 対象ファイルが 0 件（列挙が壊れている＝検査が素通りする）" >&2
    exit 2
fi

# 判定は python3 に寄せる。sed / grep のロケール依存を避けたいのと、
# 「$var の直後の 1 文字が非 ASCII か」を文字単位で見たいため。
if ! command -v python3 >/dev/null 2>&1; then
    echo "✗ python3 が無いため検査できない" >&2
    exit 2
fi

set +e
MISSING_IS_ERROR="${missing_is_error}" python3 - "${targets[@]}" <<'PY'
import os, re, sys

MISSING_IS_ERROR = os.environ.get('MISSING_IS_ERROR') == '1'

# ブレース無しの $name のみを対象にする。`${name}` は直後の文字が変数名に混ざらないので安全で、
# 後続の [A-Za-z_] が `{` に一致しないため自然に除外される。
# 位置パラメータ（$1）や特殊変数（$?）は 1 文字で終端するのでこの正規表現に一致せず、
# bash 側もそこで変数名を打ち切るため実害が無い。
VAR = re.compile(r'\$[A-Za-z_][A-Za-z0-9_]*')

hits = []
internal_error = False
for path in sys.argv[1:]:
    try:
        # デコード不能バイトは surrogateescape で保持する。定義上どれも非 ASCII なので
        # 「読めないから検査不能」ではなく検出側に倒れる（exit 2 で全体を止めない）。
        with open(path, encoding='utf-8', errors='surrogateescape') as fh:
            # splitlines() は U+2028 / U+0085 / \x0b / \x0c でも割るため bash の行概念とズレる。
            lines = fh.read().split('\n')
    except FileNotFoundError:
        if MISSING_IS_ERROR:
            # 明示的に指定されたのに無い＝タイポ・リネーム漏れ。黙って合格にしない。
            print(f'✗ {path}: 指定されたファイルが無い', file=sys.stderr)
            internal_error = True
            continue
        # 追跡済みだが作業ツリーに無い（未ステージの削除など）。検査の内部エラーではないので
        # 警告に留める——ここで exit 2 にすると pre-push が「検査が壊れた」形で止まる。
        print(f'⚠ {path}: 作業ツリーに無いのでスキップ', file=sys.stderr)
        continue
    except OSError as exc:
        # **ここで即 exit しない**——収集済みの hits を捨てると、読めないファイルが 1 本あるだけで
        # 他の全違反が報告されなくなる。フラグに溜めて最後にまとめて落とす。
        print(f'✗ {path}: 読めない（{exc}）', file=sys.stderr)
        internal_error = True
        continue
    for lineno, line in enumerate(lines, 1):
        # 行頭コメントは展開されないので除外する。これにより「この罠を説明するコメント」で
        # 悪い例（$label（ のような形）をそのまま書ける（scripts/test-check-adr-numbers.sh）。
        # **展開される複数行文脈（ヒアドキュメント本文・複数行文字列の継続行など）の # 行も
        # ここで巻き込まれる**（ヘッダの「既知の非カバー範囲」を参照）。
        if line.lstrip().startswith('#'):
            continue
        for m in VAR.finditer(line):
            end = m.end()
            if end < len(line) and ord(line[end]) > 127:
                hits.append((path, lineno, m.group(0), line[end], line.strip()))

for path, lineno, var, ch, text in hits:
    print(f'✗ {path}:{lineno}: {var} の直後に非 ASCII「{ch}」がある → ${{{var[1:]}}} と書く', file=sys.stderr)
    print(f'    {text}', file=sys.stderr)

if hits:
    # 是正ヒントは **hits があるときに必ず出す**。ラッパー側で exit code 1 のときだけ出す作りだと、
    # 読めないファイルが 1 本混ざって exit 2 になった瞬間にヒントごと消える。
    print('  UTF-8 ロケールの bash が非 ASCII を変数名に取り込み、set -u で落ちる（#636）。', file=sys.stderr)
    print('  シェルの文脈なら変数をブレースで閉じる（$var → ${var}）と解消する。', file=sys.stderr)
    print('  awk / jq / perl などを埋め込んでいる箇所は意味が変わるので、コードの組み替えで回避する。', file=sys.stderr)

if internal_error:
    sys.exit(2)
sys.exit(1 if hits else 0)
PY
code=$?
set -e

exit "${code}"
