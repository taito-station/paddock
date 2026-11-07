#!/usr/bin/env python3
"""決定ログ（knowledge/ の append-only な決定の記録）の不変性を機械検査する（#27）。

`rules/hve/artifact-management.md`「決定ログの不変性」が定める 2 方式を、方式の宣言
なしに自動判定して検査する:

- **独立ファイル方式（既定）**: `--adr-dir`（既定 `knowledge/adr`）が base コミット時点
  で存在するリポジトリに適用する。base にあった ADR ファイル（README.md を除く）は
  削除・改名を禁止する。`## ステータス` 節を除いた本文は、base と現在を
  `difflib.SequenceMatcher` の opcode で比較する。`equal` 以外の区間のうち、行数が
  変わらない `replace`（誤字修正や決定の書き換えが同じ位置の行の中だけに収まっている）
  だけを警告にする。`insert`・`delete`、または行数が変わる `replace`（機械で誤字修正と
  判定できない変更）は error にする。警告・error とも、変わった行はすべて base と現在の
  対で表示する。`## ステータス` 節自体は、見出しの数が base と現在で変わっていれば警告
  する。節の中身（ステータス値の行＝節内最初の非空行を除いた部分）が変わっている場合も
  警告する（supersede の経緯の追記等、正規の運用でも行数が変わり得るため、ここは常に
  警告にとどめ error にしない）。一覧 `README.md` は base にあった行の削除を禁止し、
  ステータス列以外の変更を警告、ステータス列の変更は許可する。README のステータス列と
  ADR 本体のステータス節の値の食い違いは、base に無い新規 ADR を含む現在の全行について
  検査し、食い違えば警告する。`--adr-dir` 配下にサブディレクトリや `.md` 以外のファイル
  （`README.md` を除く）があれば、独立ファイル方式の前提（フラット配置）から外れるため
  検査対象外になる旨を警告する（base にあったそれらの削除も同様に警告）。
  ファイル名の先頭の 4 桁連番（`NNNN-`。名前の途中の数字は番号とみなさない）が同じ ADR が
  複数あれば error にする（base に `--adr-dir` が無く、この変更で作ったときも検査する）。
  並行ブランチがどちらも main の最大値 +1 を取ると起き、ブランチ単体では検出できない
  ので、CI では main との merge commit を `--head` に渡して検査する。その番号の
  ファイルがすべて base にあった（base の時点から重複していた）なら警告にとどめる。
- **インライン方式**: `--dir`（既定 `knowledge`）配下を再帰的に見て、base 側で
  `## 決定ログ` 見出し（コードフェンスの外）を持つ `.md` を対象にする。base 時点の
  節の内容が、現在の節の先頭に一致する（prefix）ことだけを求める。末尾への追記だけが
  許され、既存行の変更・削除・途中への挿入・節や文書そのものの削除は error になる。
  1 つの文書に `## 決定ログ` 見出しが 2 つ以上あるとき、base の時点で既に重複していた
  （節の境界が base の時点からすでに一意に決まらない）なら警告にとどめる。base では
  重複していなかったのに現在重複している（base に無い新規文書での重複を含む）場合は、
  新たに生じた重複として error にする。ディレクトリの改名に伴う、節内のパス参照の
  置換は許容する。置換ペアは、改名された旧・新パスを `Path.parts` に分解し、末尾から
  共通する要素を取り除いた残り（`o`, `n`）から作る。残りを `/` で結合した full path の
  ペア（例: `src/old/sub/z.md` → `src/new/sub/z.md` なら `src/old` → `src/new`）を
  必ず作り、`o[:-1] == n[:-1] and o[-1] != n[-1]`（同じ親の下でのファイル名だけの改名）
  のときだけ、末尾成分だけのペア（例: `old` → `new`）も追加で作る（旧・新がどちらも
  1 要素なら full path のペアと同じなので作らない）。末尾成分だけのペアは、相対リンクの
  途中に出てくるディレクトリ名（例: `](../old/a.md)`）を追従させるために使うが、直後に
  `/` が続く一致だけに絞って適用し、地の文に現れる無関係な同名の語（例: 「方式 v1 を
  採用する」の `v1`）への誤爆を避ける。サブディレクトリへの移動（例: `docs/v1` →
  `docs/v1/legacy`）では親が異なるため末尾成分のペアを作らず、`](../v1/b.md)` のように
  末尾成分だけを書き換えた相対リンクは full path のペアとも一致せず error になる。
  全ペアは `(-len(old), old, new)` の順で並べ、1 本の正規表現にまとめて 1 回だけ当てる
  （ペアを順に当てると、置換後の文字列に別のペアが再びかかってしまうため）。full path
  のペアはパス区切り等で挟まれた一致だけに当てる。

比較は `git merge-base <head> <base-ref>`（既定 `<base-ref>` = `origin/main`、無ければ
`main`）を基準に行う。`--head <commit>` を渡すとそのコミットの内容と比較し、渡さない
ときは作業ツリー（未コミットの変更を含む）と比較する。作業ツリーと比較するとき、
`--adr-dir` 配下の一覧やインライン方式の新規文書検出は、追跡済みファイルと
`.gitignore` の対象外の未追跡ファイルだけを対象にする（`git ls-files --cached --others
--exclude-standard`）。`.gitignore` 対象の未追跡ファイル（`.DS_Store` 等）はノイズとして
無視する。merge-base が解決できない（ref が無い・shallow clone で履歴が足りない等）とき
は、違反の有無を判定できないので exit 2 にする（違反なしとして通さない）。

git の出力は `-c core.quotePath=false -c diff.noprefix=false -c diff.mnemonicPrefix=false`
で固定し、diff を使う箇所は `--no-ext-diff --no-textconv --no-color --text` を付ける。
内容の比較は `git diff` の文字列出力ではなく `git show <commit>:<path>`（または作業
ツリーのバイト列）を読んで行うため、`diff.external` / textconv の設定に影響されない。
パスの存在確認は `git ls-tree <commit> -- <path>` の終了コードと出力の有無だけで判定
し、「パスが無い」と「git 自体の失敗（壊れた commit 参照等）」を混同しない。
`git cat-file -e` は使わない。対象の blob オブジェクトがツリー上には残っていても物理的
に欠落・破損している場合でも「存在しない」と誤判定してしまうため（`ls-tree` ならツリー
の一覧だけで判定でき、blob の内容を読む必要がない）。内容を読む段（`git show`）が失敗
した場合は、パスの存在確認を別途行い、パスがあるのに読めない（壊れたオブジェクト等）
ときは判定不能として `git_failed()` 経由で exit 2 にする（違反なしとして通さない）。
リネーム検出（`-M`）は、インライン方式のディレクトリ改名の許容判定と、独立ファイル
方式の ADR 改名検出のためだけに使う。
`## 決定ログ` / `## ステータス` の見出しは `^##[ \t]+<見出し>[ \t]*$`（半角スペース・
タブのみ）で判定する。全角空白は見出しの区切りとして認めない（見た目は似ているが
Markdown の見出しとして機能しないため、同じものとして素通りさせない）。

使い方:
  # 既定（origin/main または main の merge-base と、作業ツリーを比較）
  python3 hve-scripts/check-decision-log.py

  # pre-push hook から: stdin で渡される push 先の commit を --head に渡す
  #   <local ref> <local sha> <remote ref> <remote sha>
  while read local_ref local_sha remote_ref remote_sha; do
      python3 hve-scripts/check-decision-log.py --head "$local_sha" || exit 1
  done

  # CI で特定のコミットを検査する
  python3 hve-scripts/check-decision-log.py --base origin/main --head "$GITHUB_SHA"

終了コード:
  0 - 違反なし（警告のみを含む場合もある）
  1 - 不変性違反（error）がある
  2 - 判定不能（merge-base が解決できない）または引数が不正

依存は標準ライブラリのみ。Python 3.9 以上で動く。
"""

import argparse
import difflib
import re
import subprocess
import sys
from pathlib import Path

GIT_CONFIG_ARGS = [
    "-c", "core.quotePath=false",
    "-c", "diff.noprefix=false",
    "-c", "diff.mnemonicPrefix=false",
]
DIFF_FIXED_ARGS = ["--no-ext-diff", "--no-textconv", "--no-color", "--text"]

RE_DECISION_LOG_HEADING = re.compile(r"^##[ \t]+決定ログ[ \t]*$")
RE_STATUS_HEADING = re.compile(r"^##[ \t]+ステータス[ \t]*$")
# 節の終端。h1 / h2 が来たらそこまで（h3 以下は節の内側）。
RE_SECTION_END = re.compile(r"^#{1,2}\s+\S")
RE_FENCE = re.compile(r"^(`{3,}|~{3,})")
RE_TABLE_ROW = re.compile(r"^\s*\|(.+)\|\s*$")
RE_TABLE_SEP_CELL = re.compile(r"^:?-{1,}:?$")
RE_NUMBER = re.compile(r"(\d{3,})")
# documentation-standards の採番（`{4桁連番}-{kebab-case-title}.md`）の連番。番号の重複検査だけが使う。
# RE_NUMBER（ステータスの突き合わせ・README の行のキー）より狭くし、名前の途中の数字を番号とみなさない（ADR 0027）
RE_ADR_FILE_NUMBER = re.compile(r"^(\d{4})-")


# --- git ヘルパー -------------------------------------------------------------


def run_git(root, *args):
    return subprocess.run(
        ["git", *GIT_CONFIG_ARGS, *args], cwd=str(root), capture_output=True, text=True
    )


def git_failed(proc, what):
    """git の失敗は判定不能として exit 2 で止める（違反なしとして通さない）。"""
    print(f"✗ git {what} に失敗した: {proc.stderr.strip()}", file=sys.stderr)
    sys.exit(2)


def repo_root():
    proc = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True
    )
    if proc.returncode != 0:
        print("✗ git リポジトリ外では実行できない", file=sys.stderr)
        sys.exit(2)
    return Path(proc.stdout.strip())


def show_blob(root, commit, rel):
    """`<commit>:<rel>` の内容を返す。パスが無ければ None。

    `git show` が失敗しても、`path_exists_at()`（`git ls-tree`）でパス自体の存在を
    別途確認する。パスがあるのに `git show` が失敗する（壊れたオブジェクト等）場合は、
    「存在しない」に寄せず git_failed() で exit 2 にする。
    """
    proc = subprocess.run(
        ["git", *GIT_CONFIG_ARGS, "show", f"{commit}:{rel}"],
        cwd=str(root),
        capture_output=True,
    )
    if proc.returncode != 0:
        if not path_exists_at(root, commit, rel):
            return None
        git_failed(
            subprocess.CompletedProcess(
                proc.args, proc.returncode, proc.stdout,
                proc.stderr.decode("utf-8", "replace"),
            ),
            f"show {commit}:{rel}",
        )
    return proc.stdout.decode("utf-8", "surrogateescape")


def read_current(root, head, rel):
    """現在（--head があればそのコミット、無ければ作業ツリー）の内容を返す。無ければ None。"""
    if head is None:
        path = root / rel
        if not path.is_file():
            return None
        return path.read_bytes().decode("utf-8", "surrogateescape")
    return show_blob(root, head, rel)


def path_exists_at(root, commit, rel):
    """`<commit>` の親ツリーに対する `git ls-tree <commit> -- <rel>` でパスの有無を判定する。

    終了コードが 0 でも出力が空なら「パスが無い」。終了コードが 0 以外なら git 自体の
    失敗（壊れた commit 参照等）として `git_failed()` 経由で exit 2 にする。
    `git cat-file -e <commit>:<rel>` は使わない。対象の blob オブジェクトがツリー上には
    残っていても物理的に欠落・破損している場合でも `cat-file -e` は「存在しない」と
    判定してしまうため（`ls-tree` はツリーの一覧だけで判定でき、blob の内容を読む必要が
    ない）。
    """
    proc = run_git(root, "ls-tree", commit, "--", rel)
    if proc.returncode != 0:
        git_failed(proc, f"ls-tree {commit} -- {rel}")
    return bool(proc.stdout.strip())


def list_md_files(root, commit, directory, recursive):
    """`directory` 配下の `.md` を列挙する（commit 時点）。"""
    norm_dir = directory.rstrip("/")
    args = ["ls-tree", "--name-only"]
    if recursive:
        args.append("-r")
    args.append(f"{commit}:{norm_dir}")
    proc = run_git(root, *args)
    if proc.returncode != 0:
        # その時点にディレクトリが無いだけなら対象なし。それ以外の失敗は判定不能として止める
        # （空として扱うと、検査を素通りさせてしまう）。
        commit_ok = run_git(root, "rev-parse", "--verify", "--quiet", f"{commit}^{{commit}}").returncode == 0
        dir_missing = not path_exists_at(root, commit, norm_dir)
        if commit_ok and dir_missing:
            return []
        git_failed(proc, f"ls-tree {commit}:{norm_dir}")
    names = []
    for line in proc.stdout.splitlines():
        line = line.strip()
        if line.endswith(".md"):
            names.append(f"{norm_dir}/{line}")
    return sorted(names)


def list_dir_tree_entries(root, commit, directory):
    """`commit` 時点の `directory` 直下のエントリを (name, kind) で返す
    （`kind` は 'blob' か 'tree'）。`directory` が無ければ空リストを返す。
    """
    norm_dir = directory.rstrip("/")
    proc = run_git(root, "ls-tree", f"{commit}:{norm_dir}")
    if proc.returncode != 0:
        commit_ok = run_git(root, "rev-parse", "--verify", "--quiet", f"{commit}^{{commit}}").returncode == 0
        dir_missing = not path_exists_at(root, commit, norm_dir)
        if commit_ok and dir_missing:
            return []
        git_failed(proc, f"ls-tree {commit}:{norm_dir}")
    entries = []
    for line in proc.stdout.splitlines():
        line = line.rstrip("\n")
        if not line:
            continue
        meta, _, name = line.partition("\t")
        fields = meta.split()
        kind = fields[1] if len(fields) > 1 else "blob"
        entries.append((name, kind))
    return entries


def list_current_dir_entries(root, head, directory):
    """`directory` 直下のエントリを (name, kind) で返す（`head` 指定時はそのコミット、
    無指定なら作業ツリー）。`kind` は 'blob' か 'tree'。

    作業ツリーの場合は、追跡済みファイルと `.gitignore` の対象外の未追跡ファイルだけを
    対象にする（`git ls-files --cached --others --exclude-standard`）。ディレクトリを
    素朴に `iterdir()` すると `.gitignore` 対象の未追跡ファイル（`.DS_Store` 等）まで
    ノイズとして拾ってしまうため使わない。
    """
    if head is None:
        return list_worktree_dir_entries(root, directory)
    return list_dir_tree_entries(root, head, directory)


def list_worktree_dir_entries(root, directory):
    """`directory` 直下のエントリを作業ツリーから (name, kind) で返す。
    追跡済みファイルと `.gitignore` の対象外の未追跡ファイルだけが対象（`kind` は
    'blob' か 'tree'。`directory` 配下にサブディレクトリがあれば、そのサブディレクトリ
    自身を 'tree' として 1 件にまとめる）。
    """
    norm_dir = directory.rstrip("/")
    proc = run_git(root, "ls-files", "--cached", "--others", "--exclude-standard", "--", norm_dir)
    if proc.returncode != 0:
        git_failed(proc, f"ls-files {norm_dir}")
    prefix = norm_dir + "/"
    entries = {}
    for line in proc.stdout.splitlines():
        line = line.strip()
        if not line.startswith(prefix):
            continue
        rel = line[len(prefix):]
        if not rel:
            continue
        head_name, _, remainder = rel.partition("/")
        if remainder:
            entries[head_name] = "tree"
        else:
            entries.setdefault(head_name, "blob")
    return sorted(entries.items())


def run_diff_name_status(root, base_sha, head, paths=None, rename=True, diff_filter=None):
    args = ["diff", *DIFF_FIXED_ARGS]
    if rename:
        args.append("-M")
    if diff_filter:
        args.append(f"--diff-filter={diff_filter}")
    args.append("--name-status")
    args.append(base_sha)
    if head is not None:
        args.append(head)
    if paths:
        args.append("--")
        args.extend(paths)
    proc = run_git(root, *args)
    if proc.returncode != 0:
        git_failed(proc, "diff --name-status")
    return [line for line in proc.stdout.splitlines() if line.strip()]


def file_rename_map(root, base_sha, head, scope_dir):
    """`scope_dir` 配下での base→現在 のファイル改名を {旧パス: 新パス} で返す。"""
    lines = run_diff_name_status(root, base_sha, head, paths=[scope_dir], rename=True, diff_filter="R")
    mapping = {}
    for line in lines:
        parts = line.split("\t")
        if len(parts) >= 3:
            mapping[parts[1]] = parts[2]
    return mapping


def detect_path_renames(root, base_sha, head):
    """base..head（or 作業ツリー）間のファイルリネームからディレクトリリネームを検出する。

    返り値は (旧, 新) のパスのリスト（`(-len(旧), 旧, 新)` の順）。base の行に適用して
    現在の行と一致すれば「パスリネームのみの変更」と判定できる。改名された旧・新パスを
    `Path.parts` に分解し、末尾から共通する要素を取り除いた残り（`o`, `n`）から置換
    ペアを作る。残りを `/` で結合した full path のペア（例: `src/old/sub/z.md` →
    `src/new/sub/z.md` なら `src/old` → `src/new`）を必ず作り、
    `o[:-1] == n[:-1] and o[-1] != n[-1]`（同じ親の下でのファイル名だけの改名）の
    ときだけ、末尾成分だけのペア（例: `old` → `new`）も追加で作る。旧・新がどちらも
    1 要素のときは full path のペアと同じになるため作らない。サブディレクトリへの移動
    （例: `docs/v1` → `docs/v1/legacy`）は親が異なるため末尾成分のペアを作らない
    （`](../v1/b.md)` のような末尾成分だけを書き換えた相対リンクへの誤爆を避ける）。
    末尾成分だけのペアは `apply_renames()` 側で「直後に `/` が続く一致だけ」に絞って
    適用し、地の文に現れる無関係な同名の語（例: 「方式 v1 を採用する」の `v1`）への
    誤爆を避ける。full path のペアはパス区切り等で挟まれた一致だけに当てる。
    リポジトリ全体を対象にする（リンク先が --dir の外にあることがあるため）。
    """
    lines = run_diff_name_status(root, base_sha, head, paths=None, rename=True, diff_filter="R")

    dir_renames = set()
    for line in lines:
        parts = line.split("\t")
        if len(parts) < 3:
            continue
        o = list(Path(parts[1]).parts)
        n = list(Path(parts[2]).parts)
        while o and n and o[-1] == n[-1]:
            o.pop()
            n.pop()
        if not o or not n:
            continue
        dir_renames.add(("/".join(o), "/".join(n)))
        if o[:-1] == n[:-1] and o[-1] != n[-1] and (len(o) > 1 or len(n) > 1):
            dir_renames.add((o[-1], n[-1]))

    return sorted(dir_renames, key=lambda p: (-len(p[0]), p[0], p[1]))


# パス境界とみなす文字（この文字で挟まれていない一致は置換しない）。英数字・`_`・`-`・`.`
# は path/word の構成要素として扱い、境界にはしない。
_NOT_BOUNDARY_CHARS = r"[\w\-.]"


def _path_boundary_pattern(token):
    """`/` を含む full path のトークンは前後とも境界で挟まれた一致だけに当てる。
    `/` を含まない単一成分のトークン（トップレベルのディレクトリ名・末尾成分）は、
    直後に `/` が続く一致だけに絞る。地の文中の単語（例: 「正本は docs」の `docs`、
    「方式 v1 を採用する」の `v1`）への誤爆を避けるため。
    """
    if "/" in token:
        return re.compile(
            rf"(?<!{_NOT_BOUNDARY_CHARS}){re.escape(token)}(?!{_NOT_BOUNDARY_CHARS})"
        )
    return re.compile(rf"(?<!{_NOT_BOUNDARY_CHARS}){re.escape(token)}(?=/)")


def apply_renames(line, renames):
    """`renames`（旧→新のペア）を、パス境界に挟まれた一致だけに、1 本の正規表現に
    まとめて 1 回だけ当てる。ペアを順に当てると、置換後の文字列に別のペアが再び
    かかってしまうため（例: `docs/v1/{a,b}.md` → `docs/v1/legacy/` の移動で、
    `docs/v1` → `docs/v1/legacy` の適用結果に同じペアが再度一致して
    `docs/legacy/legacy/a.md` になる）、全ペアを 1 本の regex の alternation に
    まとめて 1 回の `sub()` で処理する。`(-len(旧), 旧, 新)` の順で並べ、長い
    トークンを alternation の先頭に置くことで、full path のペアが末尾成分だけの
    ペアより先に試される。
    """
    if not renames:
        return line
    ordered = sorted(renames, key=lambda p: (-len(p[0]), p[0], p[1]))
    mapping = {}
    alternatives = []
    for old, new in ordered:
        if old in mapping:
            continue
        mapping[old] = new
        alternatives.append(_path_boundary_pattern(old).pattern)
    combined = re.compile("|".join(f"(?:{a})" for a in alternatives))
    return combined.sub(lambda m: mapping[m.group(0)], line)


# --- Markdown 節の抽出 ---------------------------------------------------------


def extract_section(text, heading_re):
    """`heading_re` に一致する見出し直下から次の h1/h2 まで（フェンス除外）を
    (行番号, 行末空白を落とした本文) のリストで返す。見出しが無ければ None。
    見出し行そのものは含めない。節末尾の空行は落とす。
    """
    collected = None
    fence = None
    for lineno, raw in enumerate(text.splitlines(), 1):
        stripped = raw.strip()
        opener = RE_FENCE.match(stripped)
        if opener:
            token = opener.group(1)
            if fence is None:
                fence = (token[0], len(token))
            elif token[0] == fence[0] and len(token) >= fence[1]:
                fence = None
            if collected is not None:
                collected.append((lineno, raw.rstrip()))
            continue
        if fence is None:
            if heading_re.match(raw.rstrip()):
                collected = []
                continue
            if collected is not None and RE_SECTION_END.match(raw):
                break
        if collected is not None:
            collected.append((lineno, raw.rstrip()))
    if collected is None:
        return None
    while collected and not collected[-1][1]:
        collected.pop()
    return collected


def count_headings(text, heading_re):
    """`heading_re` に一致する見出し（コードフェンスの外）の数を返す。"""
    count = 0
    fence = None
    for raw in text.splitlines():
        stripped = raw.strip()
        opener = RE_FENCE.match(stripped)
        if opener:
            token = opener.group(1)
            if fence is None:
                fence = (token[0], len(token))
            elif token[0] == fence[0] and len(token) >= fence[1]:
                fence = None
            continue
        if fence is None and heading_re.match(raw.rstrip()):
            count += 1
    return count


def strip_section(text, heading_re):
    """`heading_re` に一致する見出し行から次の h1/h2 まで（フェンス除外）を取り除いた
    残りの行リスト（行末空白を落とす）を返す。見出しが無ければ全文をそのまま返す。
    """
    result = []
    in_section = False
    fence = None
    for raw in text.splitlines():
        stripped = raw.strip()
        opener = RE_FENCE.match(stripped)
        if opener:
            token = opener.group(1)
            if fence is None:
                fence = (token[0], len(token))
            elif token[0] == fence[0] and len(token) >= fence[1]:
                fence = None
            if not in_section:
                result.append(raw.rstrip())
            continue
        if fence is None:
            if heading_re.match(raw.rstrip()):
                in_section = True
                continue
            if in_section and RE_SECTION_END.match(raw):
                in_section = False
        if not in_section:
            result.append(raw.rstrip())
    while result and not result[-1]:
        result.pop()
    return result


def format_line_pairs(base_lines, current_lines):
    """変わった行をすべて base と現在の対で表示する（対応するものが無い側は
    `(行が無い)` と表示する）。
    """
    rows = []
    for i in range(max(len(base_lines), len(current_lines))):
        b = base_lines[i] if i < len(base_lines) else None
        c = current_lines[i] if i < len(current_lines) else None
        rows.append(
            f"      - base:  {b if b is not None else '(行が無い)'}\n"
            f"      + 現在:  {c if c is not None else '(行が無い)'}"
        )
    return "\n".join(rows)


def diff_rest_outside_status(rel, base_rest, current_rest):
    """`## ステータス` 節の外の本文（行リスト）を `difflib.SequenceMatcher` の opcode
    で比較する。`equal` 以外の区間のうち、行数が変わらない `replace`（誤字修正や決定の
    書き換えが同じ位置の行の中だけに収まっている）は警告、`insert`・`delete`・行数が
    変わる `replace` は、機械で誤字修正と判定できないため error にする。警告・error
    とも、変わった行はすべて base と現在の対で表示する。
    """
    matcher = difflib.SequenceMatcher(None, base_rest, current_rest, autojunk=False)
    errors = []
    warnings = []
    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        if tag == "equal":
            continue
        base_chunk = base_rest[i1:i2]
        current_chunk = current_rest[j1:j2]
        block = format_line_pairs(base_chunk, current_chunk)
        if tag == "replace" and (i2 - i1) == (j2 - j1):
            warnings.append(
                f"{rel}: `## ステータス` 節の外で内容が変わっている（誤字修正に限る）。"
                f"base の {i1 + 1} 行目相当\n{block}"
            )
        else:
            errors.append(
                f"{rel}: `## ステータス` 節の外で行の追加・削除がある"
                f"（base の {i1 + 1} 行目相当、base {i2 - i1} 行 → 現在 {j2 - j1} 行）。"
                "行数が変わる変更・対応が取れない変更は機械では誤字修正と判定できないため"
                f"禁止する\n{block}"
            )
    return errors, warnings


def extract_status_value(text):
    """`## ステータス` 節の最初の非空行から、`— 日付` の前の値を取る。節が無ければ None。"""
    section = extract_section(text, RE_STATUS_HEADING)
    if not section:
        return None
    for _, line in section:
        line = line.strip()
        if not line:
            continue
        return re.split(r"\s+—\s+", line, maxsplit=1)[0].strip()
    return None


def status_section_lines_excluding_value(text):
    """`## ステータス` 節の行のうち、ステータス値の行（節内の最初の非空行）を除いた
    残りの行を返す（行末空白は落とす）。節が無ければ None。
    """
    section = extract_section(text, RE_STATUS_HEADING)
    if section is None:
        return None
    lines = [line for _, line in section]
    value_idx = next((i for i, line in enumerate(lines) if line.strip()), None)
    if value_idx is None:
        return lines
    return lines[:value_idx] + lines[value_idx + 1:]


def parse_tables(text):
    """テキスト中の Markdown テーブルをすべて抽出する。[(header_cells, [row_cells, ...]), ...]。"""
    lines = text.splitlines()
    tables = []
    i = 0
    n = len(lines)
    while i < n:
        header_m = RE_TABLE_ROW.match(lines[i])
        if header_m and i + 1 < n:
            sep_m = RE_TABLE_ROW.match(lines[i + 1])
            if sep_m:
                sep_cells = [c.strip() for c in sep_m.group(1).split("|")]
                if sep_cells and all(RE_TABLE_SEP_CELL.match(c) for c in sep_cells):
                    header_cells = [c.strip() for c in header_m.group(1).split("|")]
                    i += 2
                    rows = []
                    while i < n:
                        row_m = RE_TABLE_ROW.match(lines[i])
                        if not row_m:
                            break
                        rows.append([c.strip() for c in row_m.group(1).split("|")])
                        i += 1
                    tables.append((header_cells, rows))
                    continue
        i += 1
    return tables


def row_key(cells):
    first = cells[0] if cells else ""
    m = RE_NUMBER.search(first)
    return m.group(1) if m else first


def adr_number(name):
    m = RE_NUMBER.search(name)
    return m.group(1) if m else name


# --- インライン方式 ------------------------------------------------------------


def compare_prefix(rel, base_log, current_log, renames, errors):
    """違反は先頭 1 件だけ報告する（prefix 比較なので後続は連鎖的な偽陽性になるため）。"""
    for i, (base_lineno, base_line) in enumerate(base_log):
        if i >= len(current_log):
            errors.append(
                f"{rel}: 決定ログの既存エントリが削除されている"
                f"（base は {len(base_log)} 行 / 現在は {len(current_log)} 行）。"
                f"base の {base_lineno} 行目以降が消えた\n"
                f"      - base:  {base_line or '(空行)'}"
            )
            return
        current_lineno, current_line = current_log[i]
        if base_line != current_line:
            if renames and apply_renames(base_line, renames) == current_line:
                continue
            errors.append(
                f"{rel}: 決定ログの既存エントリが変更されている（{current_lineno} 行目）。"
                "決定ログは append-only で、既存の決定は書き換えず新しいエントリで覆す\n"
                f"      - base:  {base_line or '(空行)'}\n"
                f"      + 現在:  {current_line or '(空行)'}"
            )
            return


def list_worktree_md_files(root, directory):
    """作業ツリーの `directory` 配下の `.md` を再帰的に列挙する。追跡済みファイルと
    `.gitignore` の対象外の未追跡ファイルだけが対象。
    """
    norm_dir = directory.rstrip("/")
    proc = run_git(root, "ls-files", "--cached", "--others", "--exclude-standard", "--", norm_dir)
    if proc.returncode != 0:
        git_failed(proc, f"ls-files {norm_dir}")
    return sorted(
        line.strip() for line in proc.stdout.splitlines() if line.strip().endswith(".md")
    )


def list_current_md_files(root, head, directory):
    """現在（`head` 指定時はそのコミット、無指定なら作業ツリー）の `directory` 配下の
    `.md` を再帰的に列挙する。
    """
    if head is None:
        return list_worktree_md_files(root, directory)
    return list_md_files(root, head, directory, recursive=True)


def check_inline_method(root, base_sha, head, dir_, adr_dir, path_renames):
    base_files = list_md_files(root, base_sha, dir_, recursive=True)
    adr_prefix = adr_dir.rstrip("/") + "/"
    base_files = [f for f in base_files if not f.startswith(adr_prefix) and f != adr_dir]

    rename_map = file_rename_map(root, base_sha, head, dir_)

    errors = []
    warnings = []
    checked = 0
    handled_current_rels = set()
    for rel in base_files:
        base_text = show_blob(root, base_sha, rel)
        if base_text is None:
            continue
        base_log = extract_section(base_text, RE_DECISION_LOG_HEADING)
        if base_log is None:
            continue  # base に決定ログが無い＝対象外

        # G19: `## 決定ログ` 見出しの重複は、base の時点で既に重複していたなら警告に
        # とどめる（節の境界が base の時点からすでに一意に決まらない、既知の状態）。
        base_dup = count_headings(base_text, RE_DECISION_LOG_HEADING) >= 2
        if base_dup:
            warnings.append(
                f"{rel}: `## 決定ログ` 見出しが 2 つ以上ある（base）。"
                "節の境界が base の時点からすでに一意に決まらない"
            )

        current_rel = rename_map.get(rel, rel)
        handled_current_rels.add(current_rel)
        current_text = read_current(root, head, current_rel)
        if current_text is None:
            errors.append(
                f"{rel}: 決定ログを持つ文書が削除されている。"
                "決定ログは append-only で、節を持つ文書の削除はできない"
            )
            checked += 1
            continue

        current_log = extract_section(current_text, RE_DECISION_LOG_HEADING)
        if current_log is None:
            errors.append(
                f"{current_rel}: 決定ログ節ごと消えている（`## 決定ログ` の見出しが見つからない）。"
                "決定ログは append-only で、節の削除・見出しの改名はできない"
            )
            checked += 1
            continue

        # base では重複していなかったのに現在重複していれば、新たに生じた重複として
        # error にする。base で既に重複していた場合は上の警告のままにする（無関係な
        # 変更のたびにエラー化すると、既存の重複を解消するまで何もできなくなるため）。
        current_dup = count_headings(current_text, RE_DECISION_LOG_HEADING) >= 2
        if current_dup and not base_dup:
            errors.append(
                f"{current_rel}: `## 決定ログ` 見出しが 2 つ以上ある（新たに重複した）。"
                "節の境界が一意に決まらないため、1 文書に 1 節だけ許可する"
            )
            checked += 1
            continue

        checked += 1
        compare_prefix(current_rel, base_log, current_log, path_renames, errors)

    # G19: base に無い新規文書（リネーム先も含め、上のループで扱っていない文書）で
    # `## 決定ログ` 見出しが重複していれば error にする。新規文書自体は append-only
    # 検査の対象外だが、見出しの重複は新規文書でも許可しない。
    current_files = [
        f for f in list_current_md_files(root, head, dir_)
        if not f.startswith(adr_prefix) and f != adr_dir
    ]
    for rel in current_files:
        if rel in handled_current_rels:
            continue
        text = read_current(root, head, rel)
        if text is None:
            continue
        if count_headings(text, RE_DECISION_LOG_HEADING) >= 2:
            errors.append(
                f"{rel}: `## 決定ログ` 見出しが 2 つ以上ある（新規文書）。"
                "節の境界が一意に決まらないため、1 文書に 1 節だけ許可する"
            )

    return checked, errors, warnings


# --- 独立ファイル方式 ----------------------------------------------------------


def check_adr_method(root, base_sha, head, adr_dir):
    base_files = [
        f for f in list_md_files(root, base_sha, adr_dir, recursive=False)
        if Path(f).name != "README.md"
    ]
    rename_map = file_rename_map(root, base_sha, head, adr_dir)

    errors = []
    warnings = []
    checked = 0
    status_by_number = {}

    for rel in base_files:
        number = adr_number(Path(rel).name)

        if rel in rename_map:
            errors.append(
                f"{rel}: ADR ファイルが改名されている（新しいパス: {rename_map[rel]}）。"
                "独立ファイル方式はファイルの削除・改名を禁止する"
            )
            checked += 1
            continue

        current_text = read_current(root, head, rel)
        if current_text is None:
            errors.append(
                f"{rel}: ADR ファイルが削除されている。"
                "独立ファイル方式はファイルの削除・改名を禁止する"
            )
            checked += 1
            continue

        base_text = show_blob(root, base_sha, rel)

        base_status_count = count_headings(base_text, RE_STATUS_HEADING)
        current_status_count = count_headings(current_text, RE_STATUS_HEADING)
        if base_status_count != current_status_count:
            warnings.append(
                f"{rel}: `## ステータス` 節の見出しが増減した"
                f"（base: {base_status_count} 個 / 現在: {current_status_count} 個）"
            )

        base_rest = strip_section(base_text, RE_STATUS_HEADING)
        current_rest = strip_section(current_text, RE_STATUS_HEADING)
        if base_rest != current_rest:
            rest_errors, rest_warnings = diff_rest_outside_status(rel, base_rest, current_rest)
            if rest_errors:
                errors.extend(rest_errors)
                warnings.extend(rest_warnings)
                checked += 1
                continue
            warnings.extend(rest_warnings)

        # G3: `## ステータス` 節自体も、ステータス値の行（節内最初の非空行）以外の
        # 追加・削除・変更は警告にする（supersede の経緯の追記等、正規の運用でも
        # 行数が変わり得るため、ここは error にしない）。
        base_status_rest = status_section_lines_excluding_value(base_text)
        current_status_rest = status_section_lines_excluding_value(current_text)
        if (
            base_status_rest is not None
            and current_status_rest is not None
            and base_status_rest != current_status_rest
        ):
            warnings.append(
                f"{rel}: `## ステータス` 節がステータス値の行以外で変わっている"
                "（supersede の経緯の追記等は許容する）\n"
                f"{format_line_pairs(base_status_rest, current_status_rest)}"
            )

        status_by_number[number] = extract_status_value(current_text)
        checked += 1

    # F5: README との突き合わせは、base に無い新規 ADR も含めた現在の全 ADR を対象にする。
    for name, kind in list_current_dir_entries(root, head, adr_dir):
        if name == "README.md" or kind != "blob" or not name.endswith(".md"):
            continue
        number = adr_number(name)
        if number in status_by_number:
            continue
        current_text = read_current(root, head, f"{adr_dir}/{name}")
        if current_text is not None:
            status_by_number[number] = extract_status_value(current_text)

    return checked, errors, warnings, status_by_number


def check_adr_readme(root, base_sha, head, adr_dir, status_by_number):
    readme_rel = f"{adr_dir.rstrip('/')}/README.md"
    base_text = show_blob(root, base_sha, readme_rel)
    if base_text is None:
        return [], []  # base に一覧が無い→対象外

    errors = []
    warnings = []

    current_text = read_current(root, head, readme_rel)
    if current_text is None:
        errors.append(
            f"{readme_rel}: 一覧 README が削除されている。独立ファイル方式はファイルの削除を禁止する"
        )
        return errors, warnings

    current_rows = {}
    for header, rows in parse_tables(current_text):
        status_idx = next((i for i, h in enumerate(header) if h == "ステータス"), None)
        for cells in rows:
            if cells:
                current_rows[row_key(cells)] = (cells, status_idx)

    for header, rows in parse_tables(base_text):
        status_idx = next((i for i, h in enumerate(header) if h == "ステータス"), None)
        for base_cells in rows:
            if not base_cells:
                continue
            key = row_key(base_cells)
            if key not in current_rows:
                errors.append(
                    f"{readme_rel}: 一覧の行が削除されている（{base_cells[0]}）。"
                    "独立ファイル方式は一覧の行の削除を禁止する"
                )
                continue
            current_cells, _ = current_rows[key]
            for idx in range(max(len(base_cells), len(current_cells))):
                b = base_cells[idx] if idx < len(base_cells) else None
                c = current_cells[idx] if idx < len(current_cells) else None
                if b == c:
                    continue
                if status_idx is not None and idx == status_idx:
                    continue  # ステータス列の変更は許可
                warnings.append(
                    f"{readme_rel}: 一覧の行（{key}）の {idx + 1} 列目が変わっている。"
                    "ステータス列以外の変更は誤字修正に限る\n"
                    f"      - base:  {b}\n      + 現在:  {c}"
                )

    # F5: ステータス列と ADR 本体のステータス節の食い違いは、base に無い新規行も含めた
    # 現在の README の全行について検査する。
    for key, (current_cells, status_idx) in current_rows.items():
        if status_idx is None or status_idx >= len(current_cells):
            continue
        readme_status = current_cells[status_idx]
        adr_status = status_by_number.get(key)
        if adr_status is not None and readme_status != adr_status:
            warnings.append(
                f"{readme_rel}: 一覧のステータス列（{key}）が ADR 本体のステータス節と"
                f"食い違っている（README: {readme_status!r} / ADR: {adr_status!r}）"
            )

    return errors, warnings


def check_adr_dir_layout(root, base_sha, head, adr_dir):
    """独立ファイル方式はフラット配置（`.md` ファイルのみ、サブディレクトリ無し）を
    前提にする。前提から外れるエントリ（README.md を除く）があれば、検査対象外になる
    旨を警告する。base にあったそれらの削除も同様に警告する（検査対象外だったために
    その削除を本来の仕組みでは検出できないため）。
    """

    def irregular_names(entries):
        names = set()
        for name, kind in entries:
            if name == "README.md":
                continue
            if kind != "blob" or not name.endswith(".md"):
                names.add(name)
        return names

    base_irregular = irregular_names(list_dir_tree_entries(root, base_sha, adr_dir))
    current_irregular = irregular_names(list_current_dir_entries(root, head, adr_dir))

    warnings = []
    for name in sorted(current_irregular):
        warnings.append(
            f"{adr_dir}/{name}: 独立ファイル方式が前提とするフラット配置（.md ファイルのみ）"
            "から外れている。検査対象外になる"
        )
    for name in sorted(base_irregular - current_irregular):
        warnings.append(
            f"{adr_dir}/{name}: フラット配置から外れていた（検査対象外の）ファイル・"
            "ディレクトリが削除されている。検査対象外だったため、この削除自体は検出できない"
        )
    return warnings


def check_adr_numbers(root, base_sha, head, adr_dir):
    """ファイル名の先頭の 4 桁連番（`NNNN-`）が同じ ADR が複数あれば報告する。その番号のファイルがすべて
    base にあった（base の時点から重複していた）なら警告、1 本でも base に無ければ
    error にする（並行ブランチが同じ番号を取ったときは、merge ref で検査すると
    base に無い重複として出る）。
    """

    def files_by_number(entries):
        groups = {}
        for name, kind in entries:
            if name == "README.md" or kind != "blob" or not name.endswith(".md"):
                continue
            m = RE_ADR_FILE_NUMBER.match(name)
            if m:
                groups.setdefault(m.group(1), []).append(name)
        return groups

    base_groups = files_by_number(list_dir_tree_entries(root, base_sha, adr_dir))
    current_groups = files_by_number(list_current_dir_entries(root, head, adr_dir))

    errors = []
    warnings = []
    for number, names in sorted(current_groups.items()):
        if len(names) < 2:
            continue
        listed = ", ".join(f"{adr_dir}/{n}" for n in sorted(names))
        added = sorted(set(names) - set(base_groups.get(number, [])))
        if not added:
            warnings.append(
                f"同じ番号 {number} の ADR が複数ある（base の時点から）: {listed}。"
                "base の ADR は改名・削除できないので、このまま残す"
            )
        else:
            errors.append(
                f"同じ番号 {number} の ADR が複数ある: {listed}。"
                f"この変更で足した ADR（{', '.join(added)}）を、base の最大値 +1 以降の空き番号に"
                "振り直す（足した ADR だけで重複しているなら、1 本を残して振り直す）"
            )
    return errors, warnings


# --- main ----------------------------------------------------------------------


def resolve_base_ref(root, base_arg):
    if base_arg is not None:
        proc = run_git(root, "rev-parse", "--verify", "--quiet", f"{base_arg}^{{commit}}")
        if proc.returncode != 0:
            return None
        return base_arg
    for candidate in ("origin/main", "main"):
        proc = run_git(root, "rev-parse", "--verify", "--quiet", f"{candidate}^{{commit}}")
        if proc.returncode == 0:
            return candidate
    return None


def main(argv):
    parser = argparse.ArgumentParser(
        prog="check-decision-log.py",
        description="決定ログ（独立ファイル方式・インライン方式）の不変性を機械検査する。",
    )
    parser.add_argument("--dir", default="knowledge", help="インライン方式の対象ディレクトリ（既定: knowledge）")
    parser.add_argument(
        "--adr-dir", default="knowledge/adr",
        help="独立ファイル方式の ADR ディレクトリ（既定: knowledge/adr）",
    )
    parser.add_argument("--base", default=None, help="比較の基準 ref（既定: origin/main、無ければ main）")
    parser.add_argument("--head", default=None, help="比較対象のコミット（省略時は作業ツリーと比較）")
    parser.add_argument("--warn-only", action="store_true", help="違反（error）があっても exit 0 にする")
    args = parser.parse_args(argv)

    root = repo_root()

    base_ref = resolve_base_ref(root, args.base)
    if base_ref is None:
        if args.base is not None:
            print(f"✗ 比較の基準 ref を解決できない: {args.base}", file=sys.stderr)
        else:
            print("✗ 比較の基準 ref が見つからない（origin/main・main のいずれも無い）", file=sys.stderr)
        return 2

    head_for_mb = args.head if args.head else "HEAD"
    mb_proc = run_git(root, "merge-base", head_for_mb, base_ref)
    if mb_proc.returncode != 0 or not mb_proc.stdout.strip():
        print(
            f"✗ merge-base を解決できない（{head_for_mb} と {base_ref}）。"
            "shallow clone や履歴不足の可能性がある",
            file=sys.stderr,
        )
        return 2
    base_sha = mb_proc.stdout.strip()

    head = args.head
    dir_ = args.dir.rstrip("/")
    adr_dir = args.adr_dir.rstrip("/")

    errors = []
    warnings = []

    path_renames = detect_path_renames(root, base_sha, head)
    inline_checked, inline_errors, inline_warnings = check_inline_method(
        root, base_sha, head, dir_, adr_dir, path_renames
    )
    errors.extend(inline_errors)
    warnings.extend(inline_warnings)

    adr_checked = 0
    if path_exists_at(root, base_sha, adr_dir):
        adr_checked, adr_errors, adr_warnings, status_by_number = check_adr_method(
            root, base_sha, head, adr_dir
        )
        errors.extend(adr_errors)
        warnings.extend(adr_warnings)
        readme_errors, readme_warnings = check_adr_readme(
            root, base_sha, head, adr_dir, status_by_number
        )
        errors.extend(readme_errors)
        warnings.extend(readme_warnings)
        warnings.extend(check_adr_dir_layout(root, base_sha, head, adr_dir))

    # 番号の重複は、base に ADR ディレクトリが無い（この変更で作った）ときも検査する
    number_errors, number_warnings = check_adr_numbers(root, base_sha, head, adr_dir)
    errors.extend(number_errors)
    warnings.extend(number_warnings)

    for w in warnings:
        print(f"⚠ {w}", file=sys.stderr)

    if errors:
        print("", file=sys.stderr)
        for e in errors:
            print(f"✗ {e}", file=sys.stderr)
        print("", file=sys.stderr)
        print(f"✗ 決定ログの不変性違反 {len(errors)} 件（base={base_sha[:7]}）", file=sys.stderr)
        return 0 if args.warn_only else 1

    print(f"✓ 決定ログの不変性を確認（インライン {inline_checked} 本、ADR {adr_checked} 本）")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
