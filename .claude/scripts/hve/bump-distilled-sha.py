#!/usr/bin/env python3
"""knowledge 文書の `distilled_from_sha` を一括で追従する（#27 タスク 3）。

`check-knowledge.py` が STALE と報告した文書の直し方は毎回同じで「checker の出力を読む →
frontmatter の sha を手で書き換える → もう 1 コミット積む」——手数が多いほど**中身を見ずに
bump する（儀式化する）**方向へ人を押す。この道具は手数だけを削り、「本当に見直しが要るか」
の判断は人に残す。

そのため次の 2 つは**やらない**:
  - `updated` は触らない。下流の本文が実質変わったときだけ人が進める
    （`rules/hve/knowledge-maturity.md` の「`updated` の規則」）
  - 本文の差分マージはしない。STALE は「upstream が変わった」の合図でしかなく、要約を
    直すかどうかは人が読んで決める

status が `Conflict` の文書は bump しない（ADR 0011: Conflict の文書は distilled_from_sha を
蒸留前のまま保つ運用のため、放っておくと stale のまま留まるのが意図した状態）。対象に挙がって
も飛ばし、「解消待ちのため飛ばした」と報告する。

**同一コミットに自分の sha は書けない**ので、運用は必ず「本文コミット → sha 追従コミット」の
2 コミットになる。

書き込む sha は**フル 40 桁**（paddock 版の `scripts/bump-distilled-sha.py` は短縮 7 桁だったが、
ここでは廃止する）。`--sha <rev>` に短縮形や `HEAD` 等を渡しても、解決した結果のフル桁を書く
（可変参照をそのまま書くと、その文書の stale 判定が恒久的に無効化される）。

checker（`check-knowledge.py`）は `Path(__file__).resolve().parent` から呼ぶ（導入先では両方が
`.claude/scripts/hve/` に同居する）。checker 自身の引数（`--dir`・`--exclude`・`--required`・
`--allow-empty-sources-with-decision-log` 等）は、このスクリプトへの引数の後ろに `--` を置いて
続けて書くと、そのまま checker へ転送される。

**パース契約**（SessionStart hook 等の呼び出し側がこの文言をパースするので、変更する場合は
呼び出し側も直すこと）:
  - STALE が無いとき: 標準出力に「STALE な文書は無い」
  - dry-run の各行は先頭に「（dry-run）」
  - `--follow-rewritten` で追従が要る文書が無いとき: 標準出力に「追従が要る文書は無い」
  - 追従・bump で書き換えた文書: `✓ <文書>: <旧 sha> → <新 sha>`
    （create-pr Step 6.1 がこの行から書き換えた文書を拾う）

**`--follow-rewritten <base-ref>`**（squash・rebase の後の sha 追従。ADR 0014）:

squash や rebase をすると、文書の `distilled_from_sha` が指していたコミットが HEAD の履歴
から外れる。手元では通ることがある（source が PR 内で変わっていないとき。distilled_from_sha
が指す古いコミットオブジェクト自体はまだ解決できるので、祖先判定は HEAD の履歴に残っているか
どうかに関係なく成立する）が、新しく clone すると sha を解決できず CI が落ちる。逆に PR 内で
source まで変えて squash すると、手元でもすぐ STALE になる（本当の stale を隠さない）。この
モードは「本当に source が変わった stale」と「squash・rebase で sha が外れただけ」を切り分け
て、後者だけを HEAD へ追従させる。

  1. 対象候補: `git diff --name-only <base-ref>...HEAD` に出る、checker（`--dir`・`--exclude`
     の既定。`--` 以降の転送引数があればそれに従う）の対象のうち、frontmatter を持ち
     `distilled_from_sha` が空でない文書
  2. 各候補の `distilled_from_sha` について:
     - HEAD の祖先（または同一）なら何もしない
     - 祖先でなくローカルで解決できないなら、追従せず「旧 sha が見つからないため追従できない。
       蒸留し直して bump する」と報告する
     - 祖先でなく解決できる場合、`sources` の各ファイルが旧 sha と HEAD で内容が同じかを見る。
       すべて同じなら HEAD へ bump する（squash・rebase で外れただけ）。1 つでも違えば追従せず
       「旧 sha のあとに source が変わっているので追従しない。蒸留し直して bump する」と報告する
       （本当の stale を隠さないため）
  3. status が `Conflict` の文書は既存どおり飛ばす
  4. 形骸化の警告は出さない（追従では本文が変わらないのが正しい）
  5. 文書の直接指定・`--all-stale`・`--sha` とは併用できない（併用したら exit 2）

使い方:
  bump-distilled-sha.py knowledge/glossary.md [...]          # 指定文書を HEAD へ
  bump-distilled-sha.py --all-stale                          # checker が STALE と言う文書を全部
  bump-distilled-sha.py --sha 1234abcd knowledge/a.md        # sha を明示（解決してフル桁で書く）
  bump-distilled-sha.py --all-stale --dry-run                # 対象だけ見る
  bump-distilled-sha.py --all-stale -- --dir knowledge --required title,status  # checker へ転送
  bump-distilled-sha.py --follow-rewritten origin/main        # squash・rebase 後の sha 追従
  bump-distilled-sha.py --follow-rewritten origin/main --dry-run  # 追従対象だけ見る

終了コード:
  0: 正常（bump 完了、または STALE 0 件、または `--follow-rewritten` で追従が要る文書が無い）
  1: checker の他の error が残っている（bump はしたうえで報告する）、対象の不正
     （指定ファイルが無い・frontmatter に distilled_from_sha の行が無い/複数ある）、または
     `--follow-rewritten` で追従できない文書が 1 件でもある（祖先判定・旧 sha の解決・
     source 差分の確認に使う git コマンドが失敗した場合も、追従しない文書として
     ここに数える＝fail-closed）
  2: 引数不正、checker が判定不能（exit 2）、`--all-stale` が checker の出力から得た
     文書パスを root の外と解決した（G11）、または `--follow-rewritten` 自身の
     `git diff --name-only` の失敗。いずれも何も書き込まない
"""

import importlib.util
import re
import subprocess
import sys
from pathlib import Path

USAGE = __doc__

HERE = Path(__file__).resolve().parent
CHECKER = HERE / "check-knowledge.py"


def load_checker_module():
    """check-knowledge.py を import する（`--follow-rewritten` の `--dir`・`--exclude` の
    既定・フィルタ処理を checker と共有するため。モジュールは `if __name__ ==
    "__main__"` ガードの下にあるので import しても検査は走らない）。
    """
    spec = importlib.util.spec_from_file_location("check_knowledge_for_bump", CHECKER)
    assert spec is not None and spec.loader is not None, CHECKER
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# 行内だけを見る（`\s*` は改行を食うので使わない——値が空のとき次行を巻き込み、
# `distilled_from_sha:"abc"` という YAML として壊れた行を書き出す）。
# `\r?` を末尾に置くのは CRLF の文書のため。newline="" で読む（改行コードを保つ）ので、
# これが無いと CRLF 文書で 1 件もマッチせず「distilled_from_sha の行が無い」と誤報する。
RE_DISTILLED = re.compile(
    r'^(distilled_from_sha:[ \t]*)"?([^"\s#]*)"?([ \t]*(?:#.*)?\r?)$', re.MULTILINE
)
# status だけを読む（Conflict 判定用）。distilled_from_sha と同じ行内限定の形。
RE_STATUS = re.compile(
    r'^status:[ \t]*"?([^"\s#]*)"?[ \t]*(?:#.*)?\r?$', re.MULTILINE
)
# checker の STALE 行から対象文書を拾う。書式は check-knowledge.py のパース契約どおり
#   ✗ knowledge/x.md: STALE ← docs-original/... が distilled_from_sha(abc1234) より後に更新されている
# 文書・source のどちらも `(.+?)` で拾う（空白を含むパスのため。`\S+?` だと空白で
# 分断されて拾えず、--all-stale の対象から黙って漏れる。A-X1）。source 側は固定の
# 区切り文言「 が distilled_from_sha(」を手がかりに非貪欲マッチを止める。
RE_STALE_LINE = re.compile(r"^✗\s+(.+?):\s+STALE\s+←\s+(.+?)\s+が\s+distilled_from_sha\(")
# 末尾の集計行（`✗ 3 件の不整合（警告 1 件 / 解消待ち 0 件）`）。個別の error と区別する。
RE_SUMMARY_LINE = re.compile(r"^✗\s+\d+\s*件の不整合")
# `--follow-rewritten` で git に渡す前に検証する sha の形式（F3）。壊れた・偽装された
# 値をそのまま git コマンドの引数に渡さない。
RE_SHA_FORMAT = re.compile(r"^[0-9a-f]{7,40}$")


def run_git(args: "list[str]", *, cwd: "Path | None" = None) -> "subprocess.CompletedProcess[str]":
    """bump が行う git 呼び出しの単一入口（F2）。他の git 呼び出しはすべてここを通す。

    `-c core.quotePath=false` を必ず付ける。既定（quotePath=true）だと日本語などの
    非 ASCII パスが `"\\346\\227..."` という 8 進数エスケープのクォート表記で出力され、
    `git diff --name-only` の結果や終点一致の比較が外れて、`--follow-rewritten` の
    追従対象から黙って漏れる。
    `git diff` を呼ぶときは `--no-ext-diff --no-textconv` も付け、外部 diff ドライバ・
    textconv 変換に出力を歪められないようにする。
    """
    cmd = ["git", "-c", "core.quotePath=false"]
    if args and args[0] == "diff":
        cmd += [args[0], "--no-ext-diff", "--no-textconv", *args[1:]]
    else:
        cmd += list(args)
    return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)


def repo_root() -> Path:
    proc = run_git(["rev-parse", "--show-toplevel"])
    if proc.returncode != 0:
        sys.exit("git リポジトリの中で実行する")
    return Path(proc.stdout.strip())


def head_sha_full(root: Path) -> str:
    proc = run_git(["rev-parse", "HEAD"], cwd=root)
    if proc.returncode != 0:
        sys.exit("HEAD を解決できない（コミットが 1 つも無い？）")
    return proc.stdout.strip()


def stale_targets(
    root: Path, checker_args: "list[str]"
) -> "tuple[dict[str, set[str]], bool, int, str]":
    """checker を実行し、STALE と報告された文書 → その原因 sources・他の error の有無・
    checker の終了コード・出力全文を返す。

    呼び出し側が「STALE 無し」で 0 終了してよいかを判断するのに終了コードが必要
    （STALE 行が 0 本でも、他の理由で checker が落ちていれば fail-open になる）。
    """
    proc = subprocess.run(
        [sys.executable, str(CHECKER), *checker_args], cwd=root, capture_output=True, text=True
    )
    output = proc.stdout + proc.stderr
    found: "dict[str, set[str]]" = {}
    others = False
    for line in output.splitlines():
        stripped = line.strip()
        matched = RE_STALE_LINE.match(stripped)
        if matched:
            found.setdefault(matched.group(1), set()).add(matched.group(2))
        elif stripped.startswith("✗ ") and not RE_SUMMARY_LINE.match(stripped):
            others = True  # STALE 以外の error（bump では消えない）
    return found, others, proc.returncode, output


def validate_stale_targets_within_root(root: Path, reasons: "dict[str, set[str]]") -> "str | None":
    """`--all-stale` が checker の出力（STALE 行）をパースして得た文書パスが、root の中の
    ファイルに解決されることを確かめる（G11）。解決できないキーがあれば最初の 1 件を
    返す（無ければ None）。

    通常の checker は `path.relative_to(root)` で得た相対パスしか出力しないので、健全な
    動作では常に root の中に解決される。これはパース結果を無条件に信用しない防御。
    """
    for rel in reasons:
        try:
            (root / rel).resolve().relative_to(root.resolve())
        except ValueError:
            return rel
    return None


def frontmatter_span(text: str) -> "tuple[int, int] | None":
    """先頭 `---` … `---` の範囲（本文側のオフセット）を返す。無ければ None。

    **走査を frontmatter に限る**。全文を見ると、frontmatter を持たない規約文書の
    コードフェンス内テンプレートを書き換えてしまう。
    """
    lines = text.splitlines(keepends=True)
    if not lines or lines[0].strip() != "---":
        return None
    offset = len(lines[0])
    for line in lines[1:]:
        if line.strip() == "---":
            return len(lines[0]), offset
        offset += len(line)
    return None


def body_after_frontmatter(text: str) -> str:
    """frontmatter の閉じ区切り（---）以降を返す。"""
    span = frontmatter_span(text)
    if span is None:
        return text
    _, end = span
    return text[end:]


def find_distilled(text: str) -> "list[re.Match[str]]":
    """frontmatter 内の `distilled_from_sha` 行を全部返す（重複の検出用）。"""
    span = frontmatter_span(text)
    if span is None:
        return []
    start, end = span
    return list(RE_DISTILLED.finditer(text, start, end))


def find_status(text: str) -> "str | None":
    """frontmatter 内の `status` の値を返す（無ければ None）。Conflict 判定にだけ使う。"""
    span = frontmatter_span(text)
    if span is None:
        return None
    start, end = span
    match = RE_STATUS.search(text, start, end)
    return match.group(1) if match else None


def is_old_source_map_format(text: str, matched: "re.Match[str]") -> bool:
    """旧標準（source ごとのマップ）形式かを判定する（F1、ADR 0014）。

    旧形式は `distilled_from_sha:` の値を空にしたうえで、直後にインデントされた
    `<source のパス>: <sha>` の子行が続く（単一 sha ではなく source ごとに sha を持つ）。
    これをそのまま単一行の scalar として書き換えると、子行だけが取り残されて壊れた
    YAML になる（`distilled_from_sha: "<sha>"` の直後に意味のないインデント行が残る）。
    """
    if matched.group(2):
        return False  # 値が空でなければ新形式（単一 sha）
    line_no = text.count("\n", 0, matched.start())
    lines = text.splitlines()
    if line_no + 1 >= len(lines):
        return False
    next_line = lines[line_no + 1]
    if not next_line[:1] in (" ", "\t"):
        return False  # 次行がインデントされていない＝子行ではない
    stripped = next_line.strip()
    if not stripped or stripped.startswith("#") or stripped.startswith("-"):
        return False  # 空行・コメント・リスト項目は子マップではない
    return ":" in stripped


def read_doc(path: Path) -> str:
    # newline="" で改行コードを保つ。既定だと CRLF の文書が丸ごと LF に正規化され、
    # 1 行のはずの差分が全行差分に化ける。
    with path.open(encoding="utf-8", newline="") as f:
        return f.read()


def bump(path: Path, text: str, matched: "re.Match[str]", sha: str) -> "tuple[str, str] | None":
    """frontmatter の sha を書き換える。(旧 sha, 新 sha) を返す。変更不要なら None。

    `text` / `matched` は事前検証で読んだものを持ち回る（検証した内容と書く内容を
    別読みにしない）。
    """
    old = matched.group(2)
    if old == sha:
        return None
    # 値が空（`distilled_from_sha:`）の場合、group 1 はコロンで終わるので空白を補う。
    # 補わないと `distilled_from_sha:"abc"` という YAML として壊れた行を書く。
    head = matched.group(1)
    if not head.endswith((" ", "\t")):
        head += " "
    # 行末コメントの直前にも空白が要る（`"sha"# 未定` は YAML 仕様上コメントにならない）。
    tail = matched.group(3)
    if tail.startswith("#"):
        tail = " " + tail
    updated = text[: matched.start()] + f'{head}"{sha}"{tail}' + text[matched.end() :]
    with path.open("w", encoding="utf-8", newline="") as f:
        f.write(updated)
    return old, sha


def run_follow_rewritten(root: Path, base_ref: str, checker_args: "list[str]", dry_run: bool) -> int:
    """squash・rebase で履歴から外れた `distilled_from_sha` を HEAD へ追従させる（ADR 0014）。

    docstring の「`--follow-rewritten <base-ref>`」を実装する。
    """
    diff_proc = run_git(["diff", "--name-only", f"{base_ref}...HEAD"], cwd=root)
    if diff_proc.returncode != 0:
        print(
            f"git diff --name-only {base_ref}...HEAD が失敗した: {diff_proc.stderr.strip()}",
            file=sys.stderr,
        )
        return 2

    checker_mod = load_checker_module()
    parser = checker_mod.build_parser()
    try:
        ns, _ = parser.parse_known_args(checker_args)
    except SystemExit:
        print("checker への転送引数が不正", file=sys.stderr)
        return 2
    exclude_patterns = ns.exclude if ns.exclude is not None else list(checker_mod.DEFAULT_EXCLUDE)
    # checker 自身も --dir 省略時は "knowledge" にフォールバックする（argparse の
    # default は None で、既定値の解決は checker の main() 側が行う）。ここでも
    # 同じ既定値を使う。
    target_dir = (root / (ns.dir if ns.dir is not None else "knowledge")).resolve()

    # 対象候補: diff に出るファイルのうち、checker の走査対象（--dir・--exclude）に入り、
    # frontmatter を持ち distilled_from_sha が空でない文書。
    candidates: "list[tuple[str, Path, str, re.Match[str], str | None, list[str]]]" = []
    failed = 0
    for rel in sorted({line.strip() for line in diff_proc.stdout.splitlines() if line.strip()}):
        if not rel.endswith(".md"):
            continue
        path = root / rel
        if not path.is_file():
            continue  # 削除された文書は追従の対象にならない
        try:
            rel_to_dir = path.resolve().relative_to(target_dir).as_posix()
        except ValueError:
            continue  # --dir の外
        if checker_mod.is_excluded(rel_to_dir, exclude_patterns):
            continue
        text = read_doc(path)
        found = find_distilled(text)
        if not found or not found[0].group(2):
            continue
        if len(found) > 1:
            # 通常モードと揃える（G11）。checker は最後の行、bump は最初の行を見るので、
            # 放置すると追従しても STALE が消えない無言のループになる。
            print(f"✗ {rel}: frontmatter に distilled_from_sha が {len(found)} 行ある", file=sys.stderr)
            failed += 1
            continue
        status = find_status(text)
        sources = checker_mod.parse_frontmatter(text).get("sources", [])
        candidates.append((rel, path, text, found[0], status, sources))

    head_sha = head_sha_full(root)
    follow_needed = 0
    skipped_conflicts: "list[str]" = []

    for rel, path, text, matched, status, sources in candidates:
        if status == "Conflict":
            # ADR 0011: Conflict の文書は distilled_from_sha を蒸留前のまま保つ運用。
            skipped_conflicts.append(rel)
            continue

        old_sha = matched.group(2)
        if not RE_SHA_FORMAT.match(old_sha):
            # 形式が不正な値を git の引数にそのまま渡さない（F3）。
            print(f"✗ {rel}: distilled_from_sha の sha の形式が不正（{old_sha!r}）", file=sys.stderr)
            failed += 1
            continue

        is_ancestor = run_git(
            ["merge-base", "--is-ancestor", old_sha, "HEAD"], cwd=root
        ).returncode == 0
        if is_ancestor:
            continue  # HEAD の履歴に残っている。追従は不要

        resolvable = run_git(
            ["rev-parse", "--verify", f"{old_sha}^{{commit}}"], cwd=root
        ).returncode == 0
        if not resolvable:
            print(
                f"✗ {rel}: 旧 sha が見つからないため追従できない。蒸留し直して bump する",
                file=sys.stderr,
            )
            failed += 1
            continue

        # 旧 sha のあとに source が変わっていないかを確かめる（本当の stale を隠さない）。
        source_changed = False
        for src in sources:
            diff_rc = run_git(["diff", "--quiet", old_sha, "HEAD", "--", src], cwd=root).returncode
            if diff_rc != 0:
                source_changed = True
                break
        if source_changed:
            print(
                f"✗ {rel}: 旧 sha のあとに source が変わっているので追従しない。"
                f"蒸留し直して bump する",
                file=sys.stderr,
            )
            failed += 1
            continue

        if dry_run:
            print(f"（dry-run）{rel} → {head_sha}")
            follow_needed += 1
            continue

        # 形骸化検出はしない（追従では本文が変わらないのが正しい状態）。
        result = bump(path, text, matched, head_sha)
        if result is None:
            print(f"= {rel}: 既に {head_sha}（変更なし）")
        else:
            print(f"✓ {rel}: {result[0]} → {result[1]}")
        follow_needed += 1

    for rel in skipped_conflicts:
        print(f"- {rel}: 解消待ちのため飛ばした（status: Conflict）")

    if follow_needed == 0 and failed == 0:
        print("追従が要る文書は無い")

    return 1 if failed else 0


def main(argv: "list[str]") -> int:
    if argv and argv[0] in ("-h", "--help"):
        print(USAGE)
        return 0
    if not argv:
        # 引数ゼロで 0 終了すると `bump.py $(...)` が空を返したとき「何もせず成功」に
        # なる。このスクリプトの他の分岐と同じく fail-closed に倒す。
        print("引数が無い（対象の文書、または --all-stale を指定する）", file=sys.stderr)
        print(USAGE, file=sys.stderr)
        return 2

    sha = ""
    all_stale = False
    dry_run = False
    follow_rewritten = ""
    rels: "list[str]" = []
    checker_args: "list[str]" = []
    forwarding = False
    i = 0
    while i < len(argv):
        arg = argv[i]
        if forwarding:
            checker_args.append(arg)
        elif arg == "--":
            forwarding = True
        elif arg == "--all-stale":
            all_stale = True
        elif arg == "--dry-run":
            dry_run = True
        elif arg == "--sha":
            i += 1
            # 値の無い `--sha --dry-run` を許すと "--dry-run" を sha として書き込む。
            if i >= len(argv) or argv[i].startswith("-"):
                print("--sha に値が無い", file=sys.stderr)
                return 2
            sha = argv[i]
        elif arg == "--follow-rewritten":
            i += 1
            if i >= len(argv) or argv[i].startswith("-"):
                print("--follow-rewritten に値が無い", file=sys.stderr)
                return 2
            follow_rewritten = argv[i]
        elif arg.startswith("-"):
            print(f"不明なオプション: {arg}", file=sys.stderr)
            print(USAGE, file=sys.stderr)
            return 2
        else:
            rels.append(arg)
        i += 1

    root = repo_root()

    if follow_rewritten:
        if rels or all_stale or sha:
            print(
                "--follow-rewritten は文書の直接指定・--all-stale・--sha と併用できない",
                file=sys.stderr,
            )
            return 2
        return run_follow_rewritten(root, follow_rewritten, checker_args, dry_run)

    reasons: "dict[str, set[str]]" = {}
    others_remain = False
    if all_stale:
        reasons, others_remain, checker_rc, checker_output = stale_targets(root, checker_args)
        if checker_rc == 2:
            # 判定不能（shallow clone・検査対象ディレクトリ不在等）。一部の STALE が
            # 出力に混じっていても、checker 自身が全体を判定し切れていないので
            # fail-closed で止める（部分的な結果を信用して書き込まない）。
            print(
                "checker が判定不能（exit 2）で終了した。先にそちらを直す:\n"
                + checker_output.rstrip(),
                file=sys.stderr,
            )
            return 2
        escaping = validate_stale_targets_within_root(root, reasons)
        if escaping is not None:
            # G11: checker の出力をパースした文字列をそのままパスに使うため、
            # root の外を指す値を信用して読み書きしない（fail-closed）。
            print(
                f"✗ {escaping}: checker の出力から得た文書パスが root の外を指している",
                file=sys.stderr,
            )
            return 2
        rels.extend(r for r in reasons if r not in rels)
        if not rels:
            if checker_rc != 0:
                # レジストリ不在やマーカー欠落等でも出力に STALE 行は出ないので、
                # 終了コードを見ないと「直すものは無い」と誤って報告する（fail-open）。
                print(
                    "checker が STALE 以外の理由で落ちている。先にそちらを直す:\n"
                    + checker_output.rstrip(),
                    file=sys.stderr,
                )
                return 1
            print("STALE な文書は無い")
            return 0
    if not rels:
        print("対象の文書を指定する（または --all-stale）", file=sys.stderr)
        print(USAGE, file=sys.stderr)
        return 2

    if sha:
        # **解決結果を書く**。ユーザ入力をそのまま書くと `--sha HEAD` で
        # `distilled_from_sha: "HEAD"` になり、その文書の stale 判定が恒久的に無効化される
        # （HEAD は常に「今」を指すので何を変えても STALE にならない）。
        resolved = run_git(["rev-parse", "--verify", f"{sha}^{{commit}}"], cwd=root)
        if resolved.returncode != 0:
            print(f"--sha が解決できない: {sha}", file=sys.stderr)
            return 2
        target_sha = resolved.stdout.strip()
    else:
        target_sha = head_sha_full(root)

    # **書く前に全部の対象を検証する**。途中で abort すると、先行ファイルだけ書き換わった
    # 半端な状態が残り、何が起きたか読めなくなる。Conflict の文書はここで skip に回し、
    # 以降の書き込みフェーズには含めない。
    targets: "list[tuple[str, Path, str, re.Match[str]]]" = []
    skipped_conflicts: "list[str]" = []
    for rel in rels:
        candidates = [Path(rel)] if Path(rel).is_absolute() else [Path.cwd() / rel, root / rel]
        path = next((c for c in candidates if c.is_file()), None)
        if path is None:
            print(f"✗ {rel}: ファイルが無い", file=sys.stderr)
            return 1
        text = read_doc(path)

        if find_status(text) == "Conflict":
            # ADR 0011: Conflict の文書は distilled_from_sha を蒸留前のまま保つ運用。
            # 対象に挙がっても飛ばし、違反としては扱わない。
            skipped_conflicts.append(rel)
            continue

        found = find_distilled(text)
        if not found:
            print(f"✗ {rel}: frontmatter に distilled_from_sha の行が無い", file=sys.stderr)
            return 1
        if len(found) > 1:
            # checker（parse_frontmatter）は最後の行を採用し、こちらは最初の行を書く。
            # 放置すると「bump しても STALE が消えない」無言のループになる。
            print(f"✗ {rel}: frontmatter に distilled_from_sha が {len(found)} 行ある", file=sys.stderr)
            return 1
        if is_old_source_map_format(text, found[0]):
            print(
                f"✗ {rel}: 旧形式（source ごとのマップ）は扱えない。単一 sha に書き直す（ADR 0014）",
                file=sys.stderr,
            )
            return 1
        targets.append((rel, path, text, found[0]))

    for rel in skipped_conflicts:
        print(f"- {rel}: 解消待ちのため飛ばした（status: Conflict）")

    for rel, path, text, matched in targets:
        why = "".join(f"\n    ← {s}" for s in sorted(reasons.get(rel, ())))
        if dry_run:
            print(f"（dry-run）{rel} → {target_sha}{why}")
            continue
        # 形骸化検出: 「今回の bump で本文が変わったか」ではなく「前回の
        # distilled_from_sha 時点の本文と、今から書き込む本文（bump 前の現在値）が
        # 一致するか」を見る。bump() は frontmatter しか書き換えないので、前者は
        # 常に一致してしまい判定にならない。
        old_body = body_after_frontmatter(text)
        old_sha = matched.group(2)
        old_body_at_sha = None
        if old_sha:
            root_rel = path.resolve().relative_to(root.resolve()).as_posix()
            proc = run_git(["show", f"{old_sha}:{root_rel}"], cwd=root)
            if proc.returncode == 0:
                old_body_at_sha = body_after_frontmatter(proc.stdout)
        result = bump(path, text, matched, target_sha)
        if result is None:
            print(f"= {rel}: 既に {target_sha}（変更なし）")
        else:
            print(f"✓ {rel}: {result[0]} → {result[1]}{why}")
            if old_body_at_sha is not None and old_body == old_body_at_sha:
                print(
                    f"⚠ {rel}: distilled_from_sha を更新しましたが本文に変更がありません。"
                    f"形骸化していませんか？",
                    file=sys.stderr,
                )

    if not dry_run:
        print("")
        print("updated は触っていない。下流の本文が実質変わったなら手で進める。")
    if others_remain:
        print("", file=sys.stderr)
        print(
            "注意: checker には STALE 以外の error も残っている（bump では解消しない）。"
            "`check-knowledge.py` を実行して確認する。",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
