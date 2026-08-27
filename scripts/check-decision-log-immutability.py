#!/usr/bin/env python3
"""knowledge / specifications の「決定ログ」節が append-only であることを機械検査する（#652）。

ADR を廃し、決定の記録を各 knowledge / specifications の `## 決定ログ` 節へ移した（#652）。
ADR は「一度置いたら改変しない」という人手の規律で不変性を担保していたが、節に混ざった
とたんに規律だけでは守れない——同じファイルの本文は日常的に書き換わるので、その編集に
紛れて過去の決定が黙って書き換わる。ADR 時代の不変性をこのスクリプトが引き継ぐ。

判定は 1 つだけ: **base（merge-base）時点の決定ログ節が、現在の決定ログ節の先頭一致
（prefix）であること**。末尾への追記だけが許され、既存行の変更・削除・途中への挿入は
すべて error になる。

比較の前に各行の行末空白を落とし、節末尾の空行も落とす（エディタ由来の差分で落とさない）。

**既知の限界（意図的に塞いでいない）**:
  - **ファイルごと削除・リネームすると検査が消える**。走査対象が「現在存在するファイル」
    なので、base にあった決定ログはファイルが消えれば比較されない。knowledge の統合は
    運用上正当な操作（CLAUDE.md「knowledge / specifications を消す・統合するとき」）で、
    そのとき決定ログは別ファイルへ移る。移動先の追跡は本スクリプトのスコープ外。
  - 節の見出し（`## 決定ログ`）そのものを別文言へ書き換えると「節が無い」に見える。
    これは節ごと消したのと同じ扱いで error になる（下の check_file を参照）。
  - **HEAD が main 上にあるとき** merge-base は HEAD 自身になるため、main への直 push で
    入った改変は次の作業ブランチまで検出されない。ワークフロー上 main への直 push は
    禁止されているので実害はない。

依存は標準ライブラリのみ。

使い方:
  scripts/check-decision-log-immutability.py    # 検査（違反があれば 1 で終了）
"""

import re
import subprocess
import sys
from pathlib import Path

# 検査対象のディレクトリ。決定ログ節を持ちうるのはこの 2 つだけ。
TARGET_DIRS = ("docs/knowledge", "docs/specifications")

RE_DECISION_LOG_HEADING = re.compile(r"^##\s+決定ログ\s*$")
# 節の終端。h1 / h2 が来たらそこまで（h3 以下は節の内側＝各決定のエントリ見出し）。
RE_SECTION_END = re.compile(r"^#{1,2}\s+\S")
RE_FENCE = re.compile(r"^(`{3,}|~{3,})")

# git を回すディレクトリ。main() で確定させる。
_ROOT = Path(".")

# パスリネーム検出結果のキャッシュ。main() で一度だけ計算する。
_RENAMES: "list[tuple[str, str]]" = []


def git(*args: str) -> "subprocess.CompletedProcess[str]":
    return subprocess.run(["git", *args], cwd=_ROOT, capture_output=True, text=True)


def repo_root() -> Path:
    proc = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True
    )
    if proc.returncode != 0:
        sys.exit("git リポジトリ外では実行できない")
    return Path(proc.stdout.strip())


def resolve_base() -> "str | None":
    """比較の基準となるコミットを決める。決められなければ None。

    origin/main → main の順に merge-base を取り、どちらも無ければ HEAD~1 に落とす。
    浅い clone や初回コミット直後で base が取れないことはあるので、その場合は検査を
    諦める（本スクリプトの担保は「PR 内で既存行を触っていないこと」なので、比較対象が
    無いなら検査そのものが成立しない）。
    """
    for ref in ("origin/main", "main"):
        if git("rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}").returncode != 0:
            continue
        merge_base = git("merge-base", "HEAD", ref)
        if merge_base.returncode == 0 and merge_base.stdout.strip():
            return merge_base.stdout.strip()
    fallback = git("rev-parse", "--verify", "--quiet", "HEAD~1^{commit}")
    if fallback.returncode == 0 and fallback.stdout.strip():
        return fallback.stdout.strip()
    return None


def extract_decision_log(text: str) -> "list[tuple[int, str]] | None":
    """決定ログ節の中身を (行番号, 行末空白を落とした本文) で返す。節が無ければ None。

    見出し行そのものは含めない。コードフェンスの中の `## 決定ログ` は見出しと見なさない
    （規約文書が節の書き方を見本として載せるため）。節の末尾の空行は落とす。

    frontmatter は素通しでよい——`## 決定ログ` もコードフェンスも frontmatter には現れない。
    剥がすと行番号を足し戻す処理が要るぶん、間違える余地だけが増える。
    """
    collected: "list[tuple[int, str]] | None" = None
    fence: "tuple[str, int] | None" = None
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
            if RE_DECISION_LOG_HEADING.match(raw.rstrip()):
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


def detect_path_renames(base_sha: str) -> "list[tuple[str, str]]":
    """base..HEAD 間のファイルリネームからディレクトリリネームを検出する。

    返り値は (旧文字列, 新文字列) のリスト（長い順）。base の行に適用して
    current と一致すれば「パスリネームのみの変更」と判定できる。相対リンク
    （``../old-dir/``）にも対応するため、共通プレフィックスを除いた末尾
    コンポーネントも置換ペアに含める。
    """
    proc = git("diff", "--diff-filter=R", "--name-status", "-M", base_sha, "HEAD")
    if proc.returncode != 0:
        return []

    dir_renames: "set[tuple[str, str]]" = set()
    for line in proc.stdout.splitlines():
        parts = line.split("\t")
        if len(parts) < 3:
            continue
        old_dir = str(Path(parts[1]).parent)
        new_dir = str(Path(parts[2]).parent)
        if old_dir != new_dir:
            dir_renames.add((old_dir, new_dir))

    subs: "list[tuple[str, str]]" = []
    seen: "set[tuple[str, str]]" = set()
    for old_dir, new_dir in sorted(dir_renames):
        if (old_dir, new_dir) not in seen:
            subs.append((old_dir, new_dir))
            seen.add((old_dir, new_dir))
        old_parts = old_dir.split("/")
        new_parts = new_dir.split("/")
        common = 0
        for o, n in zip(old_parts, new_parts):
            if o == n:
                common += 1
            else:
                break
        if common < len(old_parts) and common < len(new_parts):
            old_suffix = "/".join(old_parts[common:])
            new_suffix = "/".join(new_parts[common:])
            if old_suffix != new_suffix and (old_suffix, new_suffix) not in seen:
                subs.append((old_suffix, new_suffix))
                seen.add((old_suffix, new_suffix))

    subs.sort(key=lambda p: len(p[0]), reverse=True)
    return subs


def apply_renames(line: str, renames: "list[tuple[str, str]]") -> str:
    """既知のパスリネームを行に適用する。"""
    result = line
    for old, new in renames:
        result = result.replace(old, new)
    return result


def blob_at(sha: str, rel: str) -> "str | None":
    proc = subprocess.run(
        ["git", "show", f"{sha}:{rel}"], cwd=_ROOT, capture_output=True
    )
    if proc.returncode != 0:
        return None
    return proc.stdout.decode("utf-8", "surrogateescape")


def check_file(rel: str, base_sha: str, path: Path, errors: list[str],
               renames: "list[tuple[str, str]] | None" = None) -> bool:
    """1 文書ぶんを検査する。比較を実施したら True（成功行の件数に数える）。

    違反は先頭 1 件だけ報告する（prefix 比較なので後続は連鎖的な偽陽性になるため）。
    """
    base_text = blob_at(base_sha, rel)
    if base_text is None:
        return False  # base に無い＝新規ファイル。全エントリが新規なので比較対象が無い
    base_log = extract_decision_log(base_text)
    if base_log is None:
        return False  # base に決定ログが無い＝全エントリが新規

    current_log = extract_decision_log(path.read_text(encoding="utf-8"))
    if current_log is None:
        errors.append(
            f"{rel}: 決定ログ節ごと消えている（`## 決定ログ` の見出しが見つからない）。"
            "決定ログは append-only で、節の削除・見出しの改名はできない"
        )
        return True

    for i, (base_lineno, base_line) in enumerate(base_log):
        if i >= len(current_log):
            errors.append(
                f"{rel}: 決定ログの既存エントリが削除されている"
                f"（base は {len(base_log)} 行 / 現在は {len(current_log)} 行）。"
                f"base の {base_lineno} 行目以降が消えた → {base_line or '(空行)'}"
            )
            return True
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
            return True
    return True


def main(argv: list[str]) -> int:
    if argv:
        print("引数は取らない", file=sys.stderr)
        return 2

    global _ROOT
    _ROOT = repo_root()

    base_sha = resolve_base()
    if base_sha is None:
        # 比較対象が無い（初回コミット / origin も main も無い）。検査は成立しないが、
        # 「違反があるのに通した」わけではないので 0 で抜ける。理由は必ず出す。
        print("比較対象のコミットが無いため決定ログの不変性検査をスキップした")
        return 0

    renames = detect_path_renames(base_sha)

    errors: list[str] = []
    checked = 0
    for directory in TARGET_DIRS:
        for path in sorted((_ROOT / directory).glob("*.md")):
            rel = path.relative_to(_ROOT).as_posix()
            if check_file(rel, base_sha, path, errors, renames):
                checked += 1

    if errors:
        print("", file=sys.stderr)
        for e in errors:
            print(f"✗ {e}", file=sys.stderr)
        print("", file=sys.stderr)
        print(f"✗ 決定ログの不変性違反 {len(errors)} 件（base={base_sha[:7]}）", file=sys.stderr)
        return 1

    print(f"✓ 決定ログの不変性を確認（{checked} 本検査）")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
