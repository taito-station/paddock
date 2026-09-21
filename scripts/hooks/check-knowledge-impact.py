#!/usr/bin/env python3
"""PostToolUse hook: docs-original / qa の編集時に影響する knowledge を警告する。

Claude Code の PostToolUse hook として実行される。
stdin から JSON（tool_name, tool_input）を受け取り、
stdout に JSON（decision）を返す。
"""
import json
import os
import re
import subprocess
import sys


def find_repo_root():
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "--show-toplevel"],
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None


def extract_file_path(hook_input):
    """tool_input からファイルパスを抽出する。"""
    tool_input = hook_input.get("tool_input") or {}
    return tool_input.get("file_path", "")


def is_docs_source(file_path, repo_root):
    """docs-original/ または qa/ 配下かどうか。"""
    rel = os.path.relpath(file_path, repo_root) if os.path.isabs(file_path) else file_path
    return rel.startswith("docs-original/") or rel.startswith("qa/")


def is_distilled_knowledge(file_path, repo_root):
    """knowledge/ または knowledge/ 配下かどうか。README.md は除外しない。"""
    rel = os.path.relpath(file_path, repo_root) if os.path.isabs(file_path) else file_path
    return rel.startswith("knowledge/") or rel.startswith("knowledge/")


def read_sources_from_frontmatter(file_path, repo_root):
    """frontmatter の sources: リストを読み取って返す。sources が無ければ空リスト。"""
    abs_path = os.path.join(repo_root, file_path) if not os.path.isabs(file_path) else file_path
    try:
        with open(abs_path, encoding="utf-8") as f:
            content = f.read()
    except OSError:
        return []

    m = re.match(r"^---\n(.*?\n)---", content, re.DOTALL)
    if not m:
        return []

    sources = []
    in_sources = False
    for line in m.group(1).splitlines():
        if line.strip().startswith("sources:"):
            in_sources = True
            continue
        if in_sources:
            if line.strip().startswith("- "):
                sources.append(line.strip()[2:].strip())
            else:
                break
    return sources


def find_affected_knowledge(file_path, repo_root):
    """file_path を sources に持つ knowledge/specifications を探す。"""
    rel_path = os.path.relpath(file_path, repo_root) if os.path.isabs(file_path) else file_path
    affected = []

    for subdir in ("knowledge",):
        dirpath = os.path.join(repo_root, subdir)
        if not os.path.isdir(dirpath):
            continue
        for fname in os.listdir(dirpath):
            if not fname.endswith(".md") or fname == "README.md":
                continue
            fpath = os.path.join(dirpath, fname)
            try:
                with open(fpath, encoding="utf-8") as f:
                    content = f.read()
            except OSError:
                continue

            m = re.match(r"^---\n(.*?\n)---", content, re.DOTALL)
            if not m:
                continue
            frontmatter_lines = m.group(1).splitlines()
            if any(line.strip().startswith("- ") and rel_path in line for line in frontmatter_lines):
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
        if is_distilled_knowledge(file_path, repo_root):
            rel_path = os.path.relpath(file_path, repo_root) if os.path.isabs(file_path) else file_path
            sources = read_sources_from_frontmatter(file_path, repo_root)
            if sources:
                sources_list = ", ".join(sources)
                message = (
                    f"⚠ SoT 逆転の可能性: {rel_path} は蒸留済み文書です（sources: {sources_list}）。"
                    f"決定ログの追記は対象外ですが、本文の変更は上流 sources を先に更新し蒸留で反映してください"
                )
                print(json.dumps({"decision": "warn", "message": message}))
                return
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
