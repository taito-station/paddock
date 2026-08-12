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
RE_DISTILLED = re.compile(
    r'^(distilled_from_sha:[ \t]*)"?([^"\s#]*)"?([ \t]*(?:#.*)?)$', re.MULTILINE
)
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
    for line in output.splitlines():
        matched = RE_STALE_LINE.match(line.strip())
        if matched:
            found.setdefault(matched.group(1), set()).add(matched.group(2))
    if not found and proc.returncode != 0:
        sys.exit(
            "checker が STALE 以外の理由で落ちている。先にそちらを直す:\n" + output.rstrip()
        )
    return found


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


def find_distilled(text: str) -> "re.Match[str] | None":
    span = frontmatter_span(text)
    if span is None:
        return None
    start, end = span
    matched = RE_DISTILLED.search(text, start, end)
    return matched


def bump(path: Path, sha: str) -> "tuple[str, str] | None":
    """frontmatter の sha を書き換える。(旧 sha, 新 sha) を返す。変更不要なら None。"""
    text = path.read_text(encoding="utf-8")
    matched = find_distilled(text)
    if not matched:
        sys.exit(f"{path}: frontmatter に distilled_from_sha の行が見つからない")
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

    if sha:
        verified = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "--verify", "--quiet", f"{sha}^{{commit}}"],
            capture_output=True,
            text=True,
        )
        if verified.returncode != 0:
            print(f"--sha が解決できない: {sha}", file=sys.stderr)
            return 2
    target_sha = sha or head_sha(root)

    # **書く前に全部の対象を検証する**。途中で abort すると、先行ファイルだけ書き換わった
    # 半端な状態が残り、何が起きたか読めなくなる。
    targets: list[tuple[str, Path]] = []
    for rel in rels:
        candidates = [Path(rel)] if Path(rel).is_absolute() else [Path.cwd() / rel, root / rel]
        path = next((c for c in candidates if c.is_file()), None)
        if path is None:
            print(f"✗ {rel}: ファイルが無い", file=sys.stderr)
            return 1
        if find_distilled(path.read_text(encoding="utf-8")) is None:
            print(f"✗ {rel}: frontmatter に distilled_from_sha の行が無い", file=sys.stderr)
            return 1
        targets.append((rel, path))

    for rel, path in targets:
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
