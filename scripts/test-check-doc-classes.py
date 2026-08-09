#!/usr/bin/env python3
"""check-doc-classes.py の回帰テスト（ADR 0073）。

同スクリプトは「ADR を knowledge へ全部写す」代わりの担保なので、壊れると stale 面積だけが
静かに増える。本番の docs/ は正常なままなので気づけない——使い捨ての fixture リポジトリを
作って各分岐の終了コードと出力を固定する。

特に重要なのは **rename-only のコミットを stale と見なさない**こと（ADR 0073 の ADR 移動だけで
20 本が一斉に誤検知する経路）。git 履歴を実際に作って検証する。

`scripts/predict-check/test_*.py` と同じ「自走式」（`def test_*()` + assert を末尾の main() が
集めて実行する）。stdlib のみ。CI の adr ジョブから `python3 <file>` で呼ばれる。

使い方:
  scripts/test-check-doc-classes.py
"""

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

TARGET = Path(__file__).resolve().parent / "check-doc-classes.py"

REGISTRY_TEMPLATE = """---
status: Confirmed
kind: knowledge
sources:
  - docs/original-docs/0001-first.md
distilled_from_sha: "{sha}"
updated: "2026-08-09"
---

# 文書クラス レジストリ（fixture）

<!-- doc-classes:begin -->
| クラス | 名称 | 状態 | 現行 |
|---|---|---|---|
| D08 | データモデル | active | {d08} |
| D12 | 権限・認可 | n/a | 0 |
| D19 | アーキテクチャ | active | {d19} |
| D22 | 予測モデル | active | {d22} |
<!-- doc-classes:end -->

<!-- doc-classes-na:begin -->
| クラス | N/A の理由 | 再開条件 |
|---|---|---|
| D12 | 認証認可を持たない | 外部公開するとき |
<!-- doc-classes-na:end -->
"""

DOC_TEMPLATE = """---
status: Confirmed
kind: knowledge
doc_class: {doc_class}
tags: {tags}
sources:
{sources}
distilled_from_sha: "{sha}"
updated: "2026-08-09"
---

# {title}

本文。
"""


def run_git(repo: Path, *args: str) -> str:
    proc = subprocess.run(
        ["git", *args], cwd=repo, capture_output=True, text=True, check=True
    )
    return proc.stdout.strip()


def commit_all(repo: Path, message: str) -> str:
    run_git(repo, "add", "-A")
    run_git(repo, "commit", "-q", "-m", message)
    return run_git(repo, "rev-parse", "--short", "HEAD")


def new_repo() -> Path:
    repo = Path(tempfile.mkdtemp(prefix="doc-classes-test-"))
    run_git(repo, "init", "-q")
    run_git(repo, "config", "user.email", "test@example.invalid")
    run_git(repo, "config", "user.name", "test")
    (repo / "docs/knowledge").mkdir(parents=True)
    (repo / "docs/specifications").mkdir(parents=True)
    (repo / "docs/original-docs").mkdir(parents=True)
    (repo / "docs/original-docs/0001-first.md").write_text(
        "# 0001. 最初の決定\n\n## ステータス\n\n承認済み。\n\n## 決定\n\nこうする。\n",
        encoding="utf-8",
    )
    return repo


def write_registry(repo: Path, sha: str, d08: int = 0, d19: int = 1, d22: int = 0) -> None:
    (repo / "docs/knowledge/doc-classes.md").write_text(
        REGISTRY_TEMPLATE.format(sha=sha, d08=d08, d19=d19, d22=d22), encoding="utf-8"
    )


def write_doc(repo: Path, rel: str, classes: list[str], sources: list[str], sha: str,
              tags: "list[str] | None" = None) -> None:
    body = DOC_TEMPLATE.format(
        doc_class="[" + ", ".join(classes) + "]",
        tags="[" + ", ".join(tags if tags is not None else classes) + "]",
        sources="\n".join(f"  - {s}" for s in sources),
        sha=sha,
        title=Path(rel).stem,
    )
    (repo / rel).write_text(body, encoding="utf-8")


def check(repo: Path, *args: str) -> "tuple[int, str]":
    proc = subprocess.run(
        [sys.executable, str(TARGET), *args], cwd=repo, capture_output=True, text=True
    )
    return proc.returncode, proc.stdout + proc.stderr


def baseline(repo: Path) -> str:
    """1 文書 + レジストリだけの、error 0 で通る状態を作って SHA を返す。"""
    write_registry(repo, "HEAD")
    write_doc(repo, "docs/knowledge/a.md", ["D19"], ["docs/original-docs/0001-first.md"], "HEAD")
    sha = commit_all(repo, "baseline")
    # frontmatter の sha を実 SHA へ差し替えて確定させる。
    write_registry(repo, sha)
    write_doc(repo, "docs/knowledge/a.md", ["D19"], ["docs/original-docs/0001-first.md"], sha)
    return commit_all(repo, "pin sha")


def test_valid_fixture_passes() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        code, out = check(repo)
        assert code == 0, f"正常系で落ちた: {out}"
        assert "✓" in out, out
    finally:
        shutil.rmtree(repo)


def test_undefined_class_is_error() -> None:
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_doc(repo, "docs/knowledge/a.md", ["D99"], ["docs/original-docs/0001-first.md"], sha)
        code, out = check(repo)
        assert code == 1, out
        assert "未定義のクラス D99" in out, out
    finally:
        shutil.rmtree(repo)


def test_na_class_is_error() -> None:
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_doc(repo, "docs/knowledge/a.md", ["D12"], ["docs/original-docs/0001-first.md"], sha)
        code, out = check(repo)
        assert code == 1, out
        assert "N/A 宣言済みのクラス D12" in out, out
    finally:
        shutil.rmtree(repo)


def test_tags_mismatch_is_error() -> None:
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_doc(repo, "docs/knowledge/a.md", ["D19"], ["docs/original-docs/0001-first.md"],
                  sha, tags=["D22"])
        code, out = check(repo)
        assert code == 1, out
        assert "tags が doc_class と一致しない" in out, out
    finally:
        shutil.rmtree(repo)


def test_tags_order_matters() -> None:
    """順序も一致を要求する（第 1 要素が主クラスなので入れ替えは意味が変わる）。"""
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_registry(repo, sha, d19=1, d22=1)
        write_doc(repo, "docs/knowledge/a.md", ["D19", "D22"],
                  ["docs/original-docs/0001-first.md"], sha, tags=["D22", "D19"])
        code, out = check(repo)
        assert code == 1, out
        assert "tags が doc_class と一致しない" in out, out
    finally:
        shutil.rmtree(repo)


def test_missing_source_is_error() -> None:
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_doc(repo, "docs/knowledge/a.md", ["D19"], ["docs/original-docs/9999-nope.md"], sha)
        code, out = check(repo)
        assert code == 1, out
        assert "sources のパスが実在しない" in out, out
    finally:
        shutil.rmtree(repo)


def test_missing_doc_class_is_error() -> None:
    repo = new_repo()
    try:
        sha = baseline(repo)
        (repo / "docs/knowledge/a.md").write_text(
            f'---\nstatus: Confirmed\nkind: knowledge\nsources:\n'
            f'  - docs/original-docs/0001-first.md\n'
            f'distilled_from_sha: "{sha}"\nupdated: "2026-08-09"\n---\n\n# a\n',
            encoding="utf-8",
        )
        code, out = check(repo)
        assert code == 1, out
        assert "doc_class が無い" in out, out
    finally:
        shutil.rmtree(repo)


def test_registry_count_mismatch_is_error() -> None:
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_registry(repo, sha, d19=99)  # 実態は 1 本
        code, out = check(repo)
        assert code == 1, out
        assert "現行列が実態と合わない" in out, out
    finally:
        shutil.rmtree(repo)


def test_na_table_inconsistency_is_error() -> None:
    """一覧の n/a と N/A 宣言表が食い違ったら落とす（片方だけ直す事故の検出）。"""
    repo = new_repo()
    try:
        sha = baseline(repo)
        text = (repo / "docs/knowledge/doc-classes.md").read_text(encoding="utf-8")
        text = text.replace("| D12 | 権限・認可 | n/a | 0 |", "| D12 | 権限・認可 | active | 0 |")
        (repo / "docs/knowledge/doc-classes.md").write_text(text, encoding="utf-8")
        code, out = check(repo)
        assert code == 1, out
        assert "N/A 宣言表にあるが一覧の状態が n/a になっていない" in out, out
    finally:
        shutil.rmtree(repo)


def test_warn_only_exits_zero() -> None:
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_doc(repo, "docs/knowledge/a.md", ["D99"], ["docs/original-docs/0001-first.md"], sha)
        code, out = check(repo, "--warn-only")
        assert code == 0, out
        assert "未定義のクラス D99" in out, out
        assert "--warn-only のため 0 で終了する" in out, out
    finally:
        shutil.rmtree(repo)


def test_stale_source_is_warning_not_error() -> None:
    """source が distilled より後に内容変更されたら stale 警告。ただし exit 0（当面 warning）。"""
    repo = new_repo()
    try:
        baseline(repo)
        p = repo / "docs/original-docs/0001-first.md"
        p.write_text(p.read_text(encoding="utf-8") + "\n追記。\n", encoding="utf-8")
        commit_all(repo, "source を実質更新")
        code, out = check(repo)
        assert code == 0, f"stale は warning のはず: {out}"
        assert "STALE" in out, out
    finally:
        shutil.rmtree(repo)


def test_rename_only_is_not_stale() -> None:
    """**最重要**: パス移動のみのコミットを stale と見なさない。

    見なすと、ディレクトリ移設（ADR 0073 の ADR 移動）だけで全 knowledge が一斉に誤検知する。
    git log --follow では吸収できないので、rename-only コミットを遡って実質の変更点を探す
    実装が要る。ここが壊れると検査全体がノイズになって誰も見なくなる。
    """
    repo = new_repo()
    try:
        baseline(repo)
        run_git(repo, "mv", "docs/original-docs/0001-first.md", "docs/original-docs/0001-moved.md")
        commit_all(repo, "パス移動のみ（内容不変）")
        # sources を新パスへ追従させる（distilled_from_sha は据え置き＝規約の例外 1）。
        sha_before = None
        for line in (repo / "docs/knowledge/a.md").read_text(encoding="utf-8").splitlines():
            if line.startswith("distilled_from_sha:"):
                sha_before = line.split('"')[1]
        assert sha_before, "fixture の distilled_from_sha を読めない"
        write_doc(repo, "docs/knowledge/a.md", ["D19"],
                  ["docs/original-docs/0001-moved.md"], sha_before)
        # レジストリ側の sources も同じファイルを指しているので併せて追従させる。
        reg = repo / "docs/knowledge/doc-classes.md"
        reg.write_text(
            reg.read_text(encoding="utf-8").replace("0001-first.md", "0001-moved.md"),
            encoding="utf-8",
        )
        commit_all(repo, "sources のパスを追従")
        code, out = check(repo)
        assert code == 0, out
        assert "STALE" not in out, f"rename-only を stale と誤判定した:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_registry_itself_needs_no_doc_class() -> None:
    """doc-classes.md 自身はクラス定義なので doc_class を要求しない（sources は検査する）。"""
    repo = new_repo()
    try:
        baseline(repo)
        code, out = check(repo)
        assert code == 0, out
        assert "doc-classes.md: doc_class が無い" not in out, out
    finally:
        shutil.rmtree(repo)


def test_readme_is_excluded() -> None:
    """README.md は規約そのもので、frontmatter のテンプレート例を含むため走査対象外。"""
    repo = new_repo()
    try:
        baseline(repo)
        (repo / "docs/knowledge/README.md").write_text(
            '---\nstatus: Confirmed\nkind: knowledge\nsources:\n'
            '  - docs/original-docs/0NNN-....md\ndistilled_from_sha: "deadbee"\n'
            'updated: "2026-08-09"\n---\n\n# README\n',
            encoding="utf-8",
        )
        code, out = check(repo)
        assert code == 0, f"README を走査してしまっている:\n{out}"
        assert "0NNN" not in out, out
    finally:
        shutil.rmtree(repo)


def test_gap_is_warning() -> None:
    """active なのに 0 本のクラスは充足ギャップとして警告（error にはしない）。"""
    repo = new_repo()
    try:
        baseline(repo)
        code, out = check(repo)
        assert code == 0, out
        assert "D08 は active だが該当文書が 0 本" in out, out
        assert "D12" not in out.split("充足ギャップ")[0] or "D12 は active" not in out, out
    finally:
        shutil.rmtree(repo)


def test_argument_handling() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        assert check(repo, "-h")[0] == 0
        assert check(repo, "--help")[0] == 0
        assert check(repo, "check")[0] == 0
        assert check(repo, "bogus")[0] == 2
        assert check(repo, "check", "extra")[0] == 2
    finally:
        shutil.rmtree(repo)


def main() -> int:
    if not TARGET.is_file():
        print(f"テスト対象が見つからない: {TARGET}", file=sys.stderr)
        return 1
    tests = [(n, f) for n, f in sorted(globals().items()) if n.startswith("test_") and callable(f)]
    failures = 0
    print("check-doc-classes.py 回帰テスト")
    for name, fn in tests:
        try:
            fn()
            print(f"  ✓ {name}")
        except AssertionError as exc:
            print(f"  ✗ {name}: {exc}", file=sys.stderr)
            failures += 1
        except Exception as exc:  # noqa: BLE001 - テスト実行時の想定外は全部落とす
            print(f"  ✗ {name}: 想定外の例外 {exc!r}", file=sys.stderr)
            failures += 1
    print("")
    if failures:
        print(f"✗ {failures} / {len(tests)} 件が失敗した", file=sys.stderr)
        return 1
    print(f"✓ 全 {len(tests)} ケース通過")
    return 0


if __name__ == "__main__":
    sys.exit(main())
