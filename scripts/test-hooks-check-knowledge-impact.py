#!/usr/bin/env python3
"""scripts/hooks/check-knowledge-impact.py の回帰テスト（#680/#683）。

この hook は PostToolUse で毎回サイレントに実行され、蒸留の抜け（docs-original/qa 編集時の
下流 knowledge 未更新）と SoT 逆転（knowledge/specifications の直接編集）を警告する。
挙動が壊れても本番の docs/ は正常なままなので気づけない——使い捨ての fixture リポジトリで
各分岐の出力を固定する。

`test-bump-distilled-sha.py` と同じ「自走式」（`def test_*()` + assert を末尾の main() が
集めて実行する）。stdlib のみ。CI の adr ジョブから `python3 <file>` で呼ばれる。

使い方:
  scripts/test-hooks-check-knowledge-impact.py
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
TARGET = HERE / "hooks" / "check-knowledge-impact.py"

KNOWLEDGE_WITH_SOURCE = """---
status: Confirmed
kind: knowledge
sources:
  - {source}
distilled_from_sha: "abc1234"
updated: "2026-08-09"
---

# fixture

本文。
"""

KNOWLEDGE_NO_SOURCES = """---
status: Confirmed
kind: knowledge
updated: "2026-08-09"
---

# fixture（sources 無し）

本文。
"""


def run_git(repo: Path, *args: str) -> None:
    subprocess.run(["git", *args], cwd=repo, capture_output=True, text=True, check=True)


def new_repo() -> Path:
    repo = Path(tempfile.mkdtemp(prefix="hook-knowledge-test-"))
    run_git(repo, "init", "-q")
    (repo / "docs/docs-original").mkdir(parents=True)
    (repo / "docs/qa").mkdir(parents=True)
    (repo / "docs/knowledge").mkdir(parents=True)
    (repo / "docs/docs-original/foo.md").write_text("# 一次資料\n\n本文。\n", encoding="utf-8")
    (repo / "docs/qa/QA-foo.md").write_text("# 質問票\n\n本文。\n", encoding="utf-8")
    (repo / "docs/knowledge/a.md").write_text(
        KNOWLEDGE_WITH_SOURCE.format(source="docs/docs-original/foo.md"), encoding="utf-8"
    )
    (repo / "docs/knowledge/b.md").write_text(
        KNOWLEDGE_WITH_SOURCE.format(source="docs/qa/QA-foo.md"), encoding="utf-8"
    )
    (repo / "docs/knowledge/no-sources.md").write_text(KNOWLEDGE_NO_SOURCES, encoding="utf-8")
    (repo / "unrelated.md").write_text("# 無関係\n\n本文。\n", encoding="utf-8")
    return repo


def run_hook(repo: Path, payload: dict) -> str:
    # git hook 経由で呼ばれると GIT_DIR が環境に残り、一時リポジトリ内の
    # `git rev-parse --show-toplevel` が別のリポジトリを指してしまう（#645 と同型の罠）。
    env = dict(os.environ)
    for key in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"):
        env.pop(key, None)
    proc = subprocess.run(
        [sys.executable, str(TARGET)],
        cwd=repo,
        input=json.dumps(payload),
        capture_output=True,
        text=True,
        env=env,
    )
    assert proc.returncode == 0, proc.stderr
    return proc.stdout.strip()


def write_payload(file_path: str, tool_name: str = "Edit") -> dict:
    return {"tool_name": tool_name, "tool_input": {"file_path": file_path}}


def test_docs_original_edit_with_downstream_warns() -> None:
    repo = new_repo()
    try:
        out = run_hook(repo, write_payload("docs/docs-original/foo.md"))
        decision = json.loads(out)
        assert decision.get("decision") == "warn", out
        assert "docs/knowledge/a.md" in decision["message"], out
    finally:
        shutil.rmtree(repo)


def test_unrelated_file_is_noop() -> None:
    repo = new_repo()
    try:
        out = run_hook(repo, write_payload("unrelated.md"))
        assert out == "{}", out
    finally:
        shutil.rmtree(repo)


def test_null_tool_input_is_noop() -> None:
    repo = new_repo()
    try:
        out = run_hook(repo, {"tool_name": "Write", "tool_input": None})
        assert out == "{}", out
    finally:
        shutil.rmtree(repo)


def test_non_write_edit_tool_is_noop() -> None:
    repo = new_repo()
    try:
        out = run_hook(repo, write_payload("docs/docs-original/foo.md", tool_name="Bash"))
        assert out == "{}", out
    finally:
        shutil.rmtree(repo)


def test_direct_knowledge_edit_with_sources_warns_sot_reversal() -> None:
    repo = new_repo()
    try:
        out = run_hook(repo, write_payload("docs/knowledge/a.md"))
        decision = json.loads(out)
        assert decision.get("decision") == "warn", out
        assert "SoT 逆転" in decision["message"], out
        assert "docs/docs-original/foo.md" in decision["message"], out
    finally:
        shutil.rmtree(repo)


def test_direct_knowledge_edit_without_sources_is_noop() -> None:
    repo = new_repo()
    try:
        out = run_hook(repo, write_payload("docs/knowledge/no-sources.md"))
        assert out == "{}", out
    finally:
        shutil.rmtree(repo)


def test_docs_qa_edit_with_downstream_warns() -> None:
    repo = new_repo()
    try:
        out = run_hook(repo, write_payload("docs/qa/QA-foo.md"))
        decision = json.loads(out)
        assert decision.get("decision") == "warn", out
        assert "docs/knowledge/b.md" in decision["message"], out
    finally:
        shutil.rmtree(repo)


def main() -> int:
    if not TARGET.is_file():
        print(f"テスト対象が見つからない: {TARGET}", file=sys.stderr)
        return 1
    tests = [(name, fn) for name, fn in sorted(globals().items())
             if name.startswith("test_") and callable(fn)]
    failed = 0
    for name, fn in tests:
        try:
            fn()
            print(f"  ✓ {name}")
        except AssertionError as e:
            failed += 1
            print(f"  ✗ {name}: {e}", file=sys.stderr)
        except Exception as e:  # noqa: BLE001 - 想定外例外も失敗として数える
            failed += 1
            print(f"  ✗ {name}: 想定外の例外 {type(e).__name__}: {e}", file=sys.stderr)
    print("")
    if failed:
        print(f"✗ {failed} / {len(tests)} 件が失敗した", file=sys.stderr)
        return 1
    print(f"✓ 全 {len(tests)} ケース通過")
    return 0


if __name__ == "__main__":
    sys.exit(main())
