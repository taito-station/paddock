#!/usr/bin/env python3
"""knowledge / specifications の `distilled_from_sha` を進める（#604）。

`sources` に挙げたファイルの本文が変わると下流は STALE=error になる。直し方は毎回同じで
「checker の出力を読む → frontmatter の sha を手で書き換える → もう 1 コミット積む」——
手数が多いほど**中身を見ずに bump する（儀式化する）**方向へ人を押す。この道具は手数だけを
削り、「本当に見直しが要るか」の判断は人に残す。

そのため次の 2 つは**やらない**:
  - `updated` は触らない。下流の本文が実質変わったときだけ人が進める（docs/knowledge/README.md）
  - 本文の差分マージはしない。STALE は「上流が変わった」の合図でしかなく、要約を直すかどうかは
    人が読んで決める

**同一コミットに自分の sha は書けない**ので、運用は必ず「本文コミット → sha 追従コミット」の
2 コミットになる。

使い方:
  scripts/bump-distilled-sha.py docs/knowledge/glossary.md [...]   # 指定文書を HEAD へ
  scripts/bump-distilled-sha.py --all-stale                        # checker が STALE と言う文書を全部
  scripts/bump-distilled-sha.py --sha 1234abc docs/knowledge/a.md  # sha を明示
  scripts/bump-distilled-sha.py --all-stale --dry-run              # 対象だけ見る
"""

import re
import subprocess
import sys
from pathlib import Path

USAGE = __doc__

CHECKER = Path(__file__).resolve().parent / "check-doc-classes.py"
# 行内だけを見る（`\s*` は改行を食うので使わない——値が空のとき次行を巻き込み、
# `distilled_from_sha:"abc"` という YAML として壊れた行を書き出す）。
# `\r?` を末尾に置くのは CRLF の文書のため。newline="" で読む（改行コードを保つ）ので、
# これが無いと CRLF 文書で 1 件もマッチせず「distilled_from_sha の行が無い」と誤報する。
RE_DISTILLED = re.compile(
    r'^(distilled_from_sha:[ \t]*)"?([^"\s#]*)"?([ \t]*(?:#.*)?\r?)$', re.MULTILINE
)
# checker の STALE 行から対象文書を拾う。書式は
#   ✗ docs/knowledge/x.md: STALE ← docs/... が distilled_from_sha(abc1234) より後に更新されている
RE_STALE_LINE = re.compile(r"^✗\s+(\S+?):\s+STALE\s+←\s+(\S+)")
# 末尾の集計行（`✗ 3 件の不整合（警告 1 件）`）。個別の error と区別する。
RE_SUMMARY_LINE = re.compile(r"^✗\s+\d+\s*件の不整合")


def repo_root() -> Path:
    proc = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True
    )
    if proc.returncode != 0:
        sys.exit("git リポジトリの中で実行する")
    return Path(proc.stdout.strip())


def head_sha(root: Path) -> str:
    proc = subprocess.run(
        ["git", "-C", str(root), "rev-parse", "--short", "HEAD"],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        sys.exit("HEAD を解決できない（コミットが 1 つも無い？）")
    return proc.stdout.strip()


def stale_targets(root: Path) -> "tuple[dict[str, set[str]], bool]":
    """checker を実行し、STALE と報告された文書 → その原因 sources を返す。

    **checker が STALE 以外の理由で落ちたときに「STALE 無し」で 0 終了しない**。
    レジストリ不在やマーカー欠落（`sys.exit`）でも出力に STALE 行は出ないので、
    終了コードを見ないと「直すものは無い」と誤って報告する（fail-open）。
    """
    proc = subprocess.run(
        [sys.executable, str(CHECKER)], cwd=root, capture_output=True, text=True
    )
    output = proc.stdout + proc.stderr
    found: dict[str, set[str]] = {}
    others = False
    for line in output.splitlines():
        stripped = line.strip()
        matched = RE_STALE_LINE.match(stripped)
        if matched:
            found.setdefault(matched.group(1), set()).add(matched.group(2))
        elif stripped.startswith("✗ ") and not RE_SUMMARY_LINE.match(stripped):
            others = True  # STALE 以外の error（bump では消えない）
    if not found and proc.returncode != 0:
        sys.exit(
            "checker が STALE 以外の理由で落ちている。先にそちらを直す:\n" + output.rstrip()
        )
    # STALE と他の error が同居することもある。bump で消えるのは STALE だけなので、
    # 「解消した」と読める 0 終了にはしない。
    return found, others


def frontmatter_span(text: str) -> "tuple[int, int] | None":
    """先頭 `---` … `---` の範囲（本文側のオフセット）を返す。無ければ None。

    **走査を frontmatter に限る**。全文を見ると、frontmatter を持たない規約文書
    （`docs/knowledge/README.md`）のコードフェンス内テンプレートを書き換えてしまう。
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


def find_distilled(text: str) -> "list[re.Match[str]]":
    """frontmatter 内の `distilled_from_sha` 行を全部返す（重複の検出用）。"""
    span = frontmatter_span(text)
    if span is None:
        return []
    start, end = span
    return list(RE_DISTILLED.finditer(text, start, end))


def read_doc(path: Path) -> str:
    # newline="" で改行コードを保つ。既定だと CRLF の文書が丸ごと LF に正規化され、
    # 1 行のはずの差分が全行差分に化ける。
    # **`Path.read_text(newline=...)` は Python 3.13 以降**なので open() を使う
    # （CI の python3 は 3.12 系。手元 3.13 だけ緑になる非対称を作らない）。
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
    # 補わないと `distilled_from_sha:"abc"` という YAML として壊れた行を書く
    # （checker の RE_SCALAR は許容するので無言で通ってしまう）。
    head = matched.group(1)
    if not head.endswith((" ", "\t")):
        head += " "
    # 行末コメントの直前にも空白が要る（`"sha"# 未定` は YAML 仕様上コメントにならない）。
    tail = matched.group(3)
    if tail.startswith("#"):
        tail = " " + tail
    updated = (
        text[: matched.start()]
        + f'{head}"{sha}"{tail}'
        + text[matched.end() :]
    )
    with path.open("w", encoding="utf-8", newline="") as f:
        f.write(updated)
    return old, sha


def main(argv: list[str]) -> int:
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
    rels: list[str] = []
    i = 0
    while i < len(argv):
        arg = argv[i]
        if arg == "--all-stale":
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
        elif arg.startswith("-"):
            print(f"不明なオプション: {arg}", file=sys.stderr)
            print(USAGE, file=sys.stderr)
            return 2
        else:
            rels.append(arg)
        i += 1

    root = repo_root()
    reasons: dict[str, set[str]] = {}
    others_remain = False
    if all_stale:
        reasons, others_remain = stale_targets(root)
        rels.extend(r for r in reasons if r not in rels)
        if not rels:
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
        resolved = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "--short", "--verify", f"{sha}^{{commit}}"],
            capture_output=True,
            text=True,
        )
        if resolved.returncode != 0:
            print(f"--sha が解決できない: {sha}", file=sys.stderr)
            return 2
        target_sha = resolved.stdout.strip()
    else:
        target_sha = head_sha(root)

    # **書く前に全部の対象を検証する**。途中で abort すると、先行ファイルだけ書き換わった
    # 半端な状態が残り、何が起きたか読めなくなる。
    targets: list[tuple[str, Path, str, "re.Match[str]"]] = []
    for rel in rels:
        candidates = [Path(rel)] if Path(rel).is_absolute() else [Path.cwd() / rel, root / rel]
        path = next((c for c in candidates if c.is_file()), None)
        if path is None:
            print(f"✗ {rel}: ファイルが無い", file=sys.stderr)
            return 1
        text = read_doc(path)
        found = find_distilled(text)
        if not found:
            print(f"✗ {rel}: frontmatter に distilled_from_sha の行が無い", file=sys.stderr)
            return 1
        if len(found) > 1:
            # checker（parse_frontmatter）は最後の行を採用し、こちらは最初の行を書く。
            # 放置すると「bump しても STALE が消えない」無言のループになる。
            print(f"✗ {rel}: frontmatter に distilled_from_sha が {len(found)} 行ある", file=sys.stderr)
            return 1
        targets.append((rel, path, text, found[0]))

    for rel, path, text, matched in targets:
        why = "".join(f"\n    ← {s}" for s in sorted(reasons.get(rel, ())))
        if dry_run:
            print(f"（dry-run）{rel} → {target_sha}{why}")
            continue
        result = bump(path, text, matched, target_sha)
        if result is None:
            print(f"= {rel}: 既に {target_sha}（変更なし）")
        else:
            print(f"✓ {rel}: {result[0]} → {result[1]}{why}")

    if not dry_run:
        print("")
        print("updated は触っていない。下流の本文が実質変わったなら手で進める。")
    if others_remain:
        print("", file=sys.stderr)
        print(
            "注意: checker には STALE 以外の error も残っている（bump では解消しない）。"
            "`scripts/check-doc-classes.py` を実行して確認する。",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
