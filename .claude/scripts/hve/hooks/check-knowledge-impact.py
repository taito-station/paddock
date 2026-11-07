#!/usr/bin/env python3
"""PostToolUse hook: docs-original / qa の編集と knowledge の直接編集を警告する（#27 タスク 4）。

Claude Code の PostToolUse hook（matcher: `Write|Edit`）として実行される。stdin から
JSON（`tool_name`・`tool_input.file_path`）を受け取り、警告があるときだけ標準出力に
1 行の JSON を返す。警告が無いときは何も出さない（PostToolUse はこの形式でないと無視
される。paddock 版が返していた `{"decision": "warn", ...}` は PostToolUse では効かない）。

警告する場面:
  1. `docs-original/` または `qa/` 配下の編集で、それを sources に挙げている knowledge
     文書がある → 「蒸留し直しが要る」
  2. `knowledge/` 配下（`knowledge/adr/` と `knowledge/README.md` を除く）の直接編集
     → 「SoT は docs-original / qa。knowledge は派生物」という SoT 逆転の注意

frontmatter の sources の読み方（`parse_frontmatter`）と除外規則（`DEFAULT_EXCLUDE` /
`is_excluded`）は `check-knowledge.py` と食い違わないよう、同スクリプトを import して使う
（自分で frontmatter パーサーを再実装しない）。checker と同じく**自分の位置から**解決する
（`Path(__file__).resolve().parent.parent / "check-knowledge.py"`。導入先では両方が
`.claude/scripts/hve/` 配下に同居する: このファイルは `hooks/` の 1 階層下）。

入力が壊れている・git リポジトリ外・checker を読み込めない等の場合は、何も出さず exit 0
にする（作業を止めない）。

出力形式（警告があるときだけ、1 行）:
  {"systemMessage": "<ユーザーに表示する文>",
   "hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": "<Claude に渡す文>"}}

settings.json への配線例（導入先が手で追加する。timeout は 10 秒程度を目安にする）:
  {
    "hooks": {
      "PostToolUse": [
        {
          "matcher": "Write|Edit",
          "hooks": [
            {
              "type": "command",
              "command": "python3 \"$CLAUDE_PROJECT_DIR\"/.claude/scripts/hve/hooks/check-knowledge-impact.py",
              "timeout": 10
            }
          ]
        }
      ]
    }
  }
"""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
from pathlib import Path


def load_checker():
    """`check-knowledge.py` をモジュールとして読み込む。失敗したら None。"""
    checker_path = Path(__file__).resolve().parent.parent / "check-knowledge.py"
    try:
        spec = importlib.util.spec_from_file_location("check_knowledge_for_hook", checker_path)
        if spec is None or spec.loader is None:
            return None
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module
    except Exception:
        return None


def find_repo_root() -> "str | None":
    try:
        proc = subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            capture_output=True,
            text=True,
        )
    except OSError:
        return None
    if proc.returncode != 0:
        return None
    return proc.stdout.strip()


def to_repo_relative(file_path: str, repo_root: str) -> "str | None":
    p = Path(file_path)
    if p.is_absolute():
        try:
            return p.resolve().relative_to(Path(repo_root).resolve()).as_posix()
        except ValueError:
            return None
    return p.as_posix()


def find_affected_knowledge(repo_root: Path, rel: str, checker) -> "list[str]":
    """`rel` を sources に挙げている knowledge 文書（repo 相対パス）を列挙する。"""
    knowledge_dir = repo_root / "knowledge"
    if not knowledge_dir.is_dir():
        return []
    affected: "list[str]" = []
    for path in sorted(knowledge_dir.rglob("*.md")):
        if not path.is_file():
            continue
        rel_to_dir = path.relative_to(knowledge_dir).as_posix()
        if checker.is_excluded(rel_to_dir, checker.DEFAULT_EXCLUDE):
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except OSError:
            continue
        fm = checker.parse_frontmatter(text)
        sources = fm.get("sources", [])
        if isinstance(sources, list) and rel in sources:
            affected.append(path.relative_to(repo_root).as_posix())
    return affected


def emit(system_message: str, additional_context: str) -> None:
    print(
        json.dumps(
            {
                "systemMessage": system_message,
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "additionalContext": additional_context,
                },
            },
            ensure_ascii=False,
        )
    )


def run() -> None:
    try:
        payload = json.load(sys.stdin)
    except (json.JSONDecodeError, UnicodeDecodeError, EOFError):
        return
    if not isinstance(payload, dict):
        return

    if payload.get("tool_name") not in ("Write", "Edit"):
        return

    tool_input = payload.get("tool_input") or {}
    if not isinstance(tool_input, dict):
        return
    file_path = tool_input.get("file_path")
    if not isinstance(file_path, str) or not file_path:
        return

    repo_root_str = find_repo_root()
    if not repo_root_str:
        return
    repo_root = Path(repo_root_str)

    rel = to_repo_relative(file_path, repo_root_str)
    if rel is None:
        return

    checker = load_checker()
    if checker is None:
        return

    if rel.startswith("docs-original/") or rel.startswith("qa/"):
        affected = find_affected_knowledge(repo_root, rel, checker)
        if not affected:
            return
        files_list = ", ".join(affected)
        emit(
            f"⚠ {rel} は knowledge の source です。蒸留し直しが必要: {files_list}",
            f"{rel} を編集した。これを sources に挙げている knowledge 文書（{files_list}）は"
            "蒸留し直しが必要（AKM 等で本文と distilled_from_sha を更新する）。",
        )
        return

    if rel.startswith("knowledge/"):
        rel_to_knowledge = rel[len("knowledge/") :]
        if checker.is_excluded(rel_to_knowledge, checker.DEFAULT_EXCLUDE):
            return
        emit(
            f"⚠ {rel} を直接編集した。SoT は docs-original / qa。knowledge は派生物",
            f"{rel} は蒸留済みの knowledge 文書（派生物）。SoT（source of truth）は "
            "docs-original/ と qa/ であり、本文の実質変更は先にそちらを更新してから"
            "蒸留で反映する。",
        )
        return


def main() -> int:
    try:
        run()
    except Exception:
        # 入力やリポジトリ状態がどう壊れていても hook は作業を止めない。
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
