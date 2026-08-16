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
# 2026-08-16 に deployments/launchd/uninstall.sh がこれで異常終了し、kill は成功したのに
# 後続の lock 削除に到達しないという「最後まで走ったように見えて走っていない」状態になった。
#
# **さらに悪いことに、この地雷は失敗報告の行に集中しやすい**（`✗ $name（…）` のような書き方）。
# 何が失敗したかを伝えるはずのメッセージが、まさにその場面で消える。
#
# `shellcheck 0.11.0` はこれを検出しない（--severity=style でも exit 0）ので専用の検査を置く。
# 実行時の挙動はロケールとプラットフォームに依存して確かめにくいため、**静的に字面で禁じる**。
#
# **この検査自体の回帰テストは scripts/test-check-shell-var-brace.sh**（リポジトリは常に合格側なので、
# 検査が壊れても本番データが正常だと無言で緑になる。ADR 0073 と同じ理由でテストを持つ）。
set -euo pipefail

if ! root="$(git rev-parse --show-toplevel 2>/dev/null)"; then
    echo "git リポジトリ外では実行できない（対象ファイルの列挙に git ls-files を使う）" >&2
    exit 1
fi
cd "$root"

# 対象は CI の shellcheck ジョブ（.github/workflows/ci.yml）と同一集合にする。
# 二重管理を作らないため、追加・変更するときは両方を揃える。
# mapfile は bash 4+ の組み込みで **macOS の bash 3.2 には無い**（本検査が守ろうとしている環境そのもの）。
# 読み込みループで代替する。
targets=()
while IFS= read -r target; do
    targets+=("$target")
done < <(git ls-files '*.sh' scripts/mdq scripts/git-hooks/pre-push)

if [ "${#targets[@]}" -eq 0 ]; then
    echo "✗ 対象ファイルが 0 件（git ls-files の指定が壊れている＝検査が素通りする）" >&2
    exit 1
fi

# 判定は python3 に寄せる。sed / grep のロケール依存を避けたいのと、
# 「$var の直後の 1 文字が非 ASCII か」を文字単位で見たいため。
if ! command -v python3 >/dev/null 2>&1; then
    echo "✗ python3 が無いため検査できない" >&2
    exit 1
fi

if ! python3 - "${targets[@]}" <<'PY'
import re, sys

# ブレース無しの $name のみを対象にする。${name} は安全なので拾わない。
# 位置パラメータ（$1）や特殊変数（$?）は 1 文字で終端するのでこの正規表現に一致せず、
# bash 側もそこで変数名を打ち切るため実害が無い。
VAR = re.compile(r'\$(?!\{)[A-Za-z_][A-Za-z0-9_]*')

hits = []
for path in sys.argv[1:]:
    try:
        with open(path, encoding='utf-8') as fh:
            lines = fh.read().splitlines()
    except (OSError, UnicodeDecodeError) as exc:
        print(f'✗ {path}: 読めない（{exc}）', file=sys.stderr)
        sys.exit(1)
    for lineno, line in enumerate(lines, 1):
        # 行頭コメントは展開されないので除外する。これにより「この罠を説明するコメント」で
        # 悪い例（$label（ のような形）をそのまま書ける（scripts/test-check-adr-numbers.sh）。
        # 行末コメントは追わない——シェルの字句解析が要るうえ、コメントでもブレースを
        # 付けておいて損は無いため、検出側に倒す。
        if line.lstrip().startswith('#'):
            continue
        for m in VAR.finditer(line):
            end = m.end()
            if end < len(line) and ord(line[end]) > 127:
                hits.append((path, lineno, m.group(0), line[end], line.strip()))

for path, lineno, var, ch, text in hits:
    print(f'✗ {path}:{lineno}: {var} の直後に非 ASCII「{ch}」がある → ${{{var[1:]}}} と書く', file=sys.stderr)
    print(f'    {text}', file=sys.stderr)

sys.exit(1 if hits else 0)
PY
then
    echo "  UTF-8 ロケールの bash が非 ASCII を変数名に取り込み、set -u で落ちる（#636）。" >&2
    echo "  変数をブレースで閉じる（\$var → \${var}）と解消する。" >&2
    exit 1
fi
