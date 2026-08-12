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
RE_DISTILLED = re.compile(r'^(distilled_from_sha:\s*)"?([^"\s#]*)"?(\s*(?:#.*)?)$', re.MULTILINE)
# checker の STALE 行から対象文書を拾う。書式は
#   ✗ docs/knowledge/x.md: STALE ← docs/... が distilled_from_sha(abc1234) より後に更新されている
RE_STALE_LINE = re.compile(r"^✗\s+(\S+?):\s+STALE\s+←\s+(\S+)")


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


def stale_targets(root: Path) -> "dict[str, set[str]]":
    """checker を実行し、STALE と報告された文書 → その原因 sources を返す。"""
    proc = subprocess.run(
        [sys.executable, str(CHECKER)], cwd=root, capture_output=True, text=True
    )
    found: dict[str, set[str]] = {}
    for line in (proc.stdout + proc.stderr).splitlines():
        matched = RE_STALE_LINE.match(line.strip())
        if matched:
            found.setdefault(matched.group(1), set()).add(matched.group(2))
    return found


def bump(path: Path, sha: str) -> "tuple[str, str] | None":
    """frontmatter の sha を書き換える。(旧 sha, 新 sha) を返す。変更不要なら None。"""
    text = path.read_text(encoding="utf-8")
    matched = RE_DISTILLED.search(text)
    if not matched:
        sys.exit(f"{path}: distilled_from_sha の行が見つからない")
    old = matched.group(2)
    if old == sha:
        return None
    updated = (
        text[: matched.start()]
        + f'{matched.group(1)}"{sha}"{matched.group(3)}'
        + text[matched.end() :]
    )
    path.write_text(updated, encoding="utf-8")
    return old, sha


def main(argv: list[str]) -> int:
    if not argv or argv[0] in ("-h", "--help"):
        print(USAGE)
        return 0

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
            if i >= len(argv):
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
    if all_stale:
        reasons = stale_targets(root)
        rels.extend(r for r in reasons if r not in rels)
        if not rels:
            print("STALE な文書は無い")
            return 0
    if not rels:
        print("対象の文書を指定する（または --all-stale）", file=sys.stderr)
        print(USAGE, file=sys.stderr)
        return 2

    target_sha = sha or head_sha(root)
    for rel in rels:
        path = (root / rel) if not Path(rel).is_absolute() else Path(rel)
        if not path.is_file():
            print(f"✗ {rel}: ファイルが無い", file=sys.stderr)
            return 1
        why = "".join(f"\n    ← {s}" for s in sorted(reasons.get(rel, ())))
        if dry_run:
            print(f"（dry-run）{rel} → {target_sha}{why}")
            continue
        result = bump(path, target_sha)
        if result is None:
            print(f"= {rel}: 既に {target_sha}（変更なし）")
        else:
            print(f"✓ {rel}: {result[0]} → {result[1]}{why}")

    if not dry_run:
        print("")
        print("updated は触っていない。下流の本文が実質変わったなら手で進める。")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
