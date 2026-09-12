#!/usr/bin/env python3
"""PostToolUse hook: docs-original / qa の編集時に影響する knowledge を警告する。

Claude Code の PostToolUse hook として実行される。
stdin から JSON（tool_name, tool_input）を受け取り、
stdout に JSON（decision）を返す。
"""
import json
import os
import re
import sys


def find_repo_root():
    d = os.path.dirname(os.path.abspath(__file__))
    while d != os.path.dirname(d):
        if os.path.isdir(os.path.join(d, ".git")):
            return d
        d = os.path.dirname(d)
    return None


def extract_file_path(hook_input):
    """tool_input からファイルパスを抽出する。"""
    tool_input = hook_input.get("tool_input", {})
    return tool_input.get("file_path", "")


def is_docs_source(file_path, repo_root):
    """docs-original/ または docs/qa/ 配下かどうか。"""
    rel = os.path.relpath(file_path, repo_root) if os.path.isabs(file_path) else file_path
    return rel.startswith("docs/docs-original/") or rel.startswith("docs/qa/")


def find_affected_knowledge(file_path, repo_root):
    """file_path を sources に持つ knowledge/specifications を探す。"""
    rel_path = os.path.relpath(file_path, repo_root) if os.path.isabs(file_path) else file_path
    affected = []

    for subdir in ("docs/knowledge", "docs/specifications"):
        dirpath = os.path.join(repo_root, subdir)
        if not os.path.isdir(dirpath):
            continue
        for fname in os.listdir(dirpath):
            if not fname.endswith(".md") or fname == "README.md":
                continue
            fpath = os.path.join(dirpath, fname)
            try:
                with open(fpath, encoding="utf-8") as f:
                    content = f.read(4096)
            except OSError:
                continue

            m = re.match(r"^---\n(.*?\n)---", content, re.DOTALL)
            if not m:
                continue
            frontmatter = m.group(1)
            if rel_path in frontmatter:
                affected.append(os.path.join(subdir, fname))

    return affected


def main():
    try:
        hook_input = json.load(sys.stdin)
    except (json.JSONDecodeError, EOFError):
        print("{}")
        return

    tool_name = hook_input.get("tool_name", "")
    if tool_name not in ("Write", "Edit"):
        print("{}")
        return

    file_path = extract_file_path(hook_input)
    if not file_path:
        print("{}")
        return

    repo_root = find_repo_root()
    if not repo_root:
        print("{}")
        return

    if not is_docs_source(file_path, repo_root):
        print("{}")
        return

    affected = find_affected_knowledge(file_path, repo_root)
    if not affected:
        print("{}")
        return

    rel_path = os.path.relpath(file_path, repo_root) if os.path.isabs(file_path) else file_path
    files_list = ", ".join(affected)
    message = (
        f"⚠ {rel_path} は以下の knowledge の source です。蒸留が必要になります: {files_list}"
    )

    print(json.dumps({"decision": "warn", "message": message}))


if __name__ == "__main__":
    main()
