#!/usr/bin/env python3
"""ADR 本文を knowledge / specifications の「決定ログ」節へ移設する片道移行スクリプト（#652）。

このスクリプトは #652 の一括移行で使用済み。再実行の必要はない。

--dry-run（既定）: ADR → 移設先のマッピングを JSON で stdout に出して終了（ファイルは触らない）
--execute        : 実際に移設する（決定ログ追記 / sources 掃除 / 本文リンク書き換え / ADR 削除）
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
ORIGINAL_DOCS = REPO_ROOT / "docs" / "original-docs"
TARGET_DIRS = [REPO_ROOT / "docs" / "knowledge", REPO_ROOT / "docs" / "specifications"]

ADR_FILENAME_RE = re.compile(r"^(0\d{3})-.*\.md$")
ADR_SOURCE_RE = re.compile(r"^docs/original-docs/0\d{3}-.*\.md$")

# 横断索引であって「話題の家」ではないので、複数参照時の第一候補から外す
CROSS_CUTTING = {"docs/knowledge/glossary.md", "docs/knowledge/product-goals.md"}

# どの sources にも載らない例外 ADR（doc-classes.md の adr-orphan-exceptions 表）
ORPHAN_FALLBACK = {"0074": "docs/knowledge/README.md"}

# アルファベット順タイブレークがトピックに合わない ADR の手動オーバーライド
MANUAL_OVERRIDE = {
    "0055": "docs/specifications/learned-model-harness.md",
    "0060": "docs/specifications/live-ev-buy-view.md",
    "0064": "docs/specifications/live-ev-buy-view.md",
}

DECISION_LOG_HEADER = (
    "\n---\n\n## 決定ログ\n\n"
    "<!-- この節は append-only です。既存エントリの変更・削除は CI が検出します。 -->\n\n"
)

# `# 0055. タイトル` と `# ADR 0001: タイトル` の 2 系統が実在する
ADR_TITLE_RE = re.compile(r"^#\s+(?:ADR\s+)?(0\d{3})\s*[.:]\s*(.+?)\s*$")

ADR_LINK_RE = re.compile(r"\[([^\]]*)\]\(([^)]*original-docs/0[^)]*)\)")

FENCE_RE = re.compile(r"^\s*(```|~~~)")


def log(msg: str) -> None:
    print(msg, file=sys.stderr)


def split_frontmatter(text: str) -> tuple[list[str] | None, str]:
    """(frontmatter 行リスト, body) を返す。frontmatter が無ければ (None, text)。"""
    lines = text.split("\n")
    if not lines or lines[0].strip() != "---":
        return None, text
    for i in range(1, len(lines)):
        if lines[i].strip() == "---":
            return lines[1:i], "\n".join(lines[i + 1 :])
    return None, text


def join_frontmatter(fm_lines: list[str], body: str) -> str:
    return "---\n" + "\n".join(fm_lines) + "\n---\n" + body


def find_block(fm_lines: list[str], key: str) -> tuple[int, int] | None:
    """`key:` の行番号と、その配下の `  - ` 項目を含めた終端 (start, end_exclusive) を返す。"""
    start = None
    for i, line in enumerate(fm_lines):
        if re.match(rf"^{re.escape(key)}\s*:", line):
            start = i
            break
    if start is None:
        return None
    end = start + 1
    while end < len(fm_lines):
        line = fm_lines[end]
        if line.strip().startswith("#") or re.match(r"^\s+-\s+", line) or not line.strip():
            # 空行は配下と見なさない（次キーの前の余白を食わないため）
            if not line.strip():
                break
            end += 1
            continue
        break
    return start, end


def parse_sources(fm_lines: list[str]) -> list[str]:
    block = find_block(fm_lines, "sources")
    if block is None:
        return []
    start, end = block
    items = []
    for line in fm_lines[start + 1 : end]:
        m = re.match(r"^\s+-\s+(.*?)\s*$", line)
        if m:
            items.append(m.group(1).strip().strip("\"'"))
    return items


def target_files() -> list[Path]:
    files: list[Path] = []
    for d in TARGET_DIRS:
        files.extend(sorted(p for p in d.glob("*.md")))
    return files


def rel(p: Path) -> str:
    return p.relative_to(REPO_ROOT).as_posix()


def adr_files() -> dict[str, Path]:
    out = {}
    for p in sorted(ORIGINAL_DOCS.glob("*.md")):
        m = ADR_FILENAME_RE.match(p.name)
        if m:
            out[m.group(1)] = p
    return out


def adr_status(path: Path) -> str:
    """`## ステータス` 節の本文全体を返す（複数行のものが 90 本中 23 本ある）。"""
    lines = path.read_text(encoding="utf-8").split("\n")
    for i, line in enumerate(lines):
        if line.strip() != "## ステータス":
            continue
        buf: list[str] = []
        for nxt in lines[i + 1 :]:
            if re.match(r"^#{1,6}\s", nxt):
                break
            buf.append(nxt)
        return "\n".join(buf).strip("\n")
    return ""


def status_label(status: str) -> str:
    """見出しに載せる短いステータス語（採用 / 棄却 / 承認済み …）を切り出す。"""
    if not status:
        return ""
    first = status.split("\n")[0].replace("**", "").strip()
    label = re.split(r"[（(。]|——", first)[0].strip()
    return label or first


def build_mapping(adrs: dict[str, Path]) -> tuple[dict[str, str], dict[str, list[str]]]:
    refs: dict[str, list[str]] = {num: [] for num in adrs}
    for f in target_files():
        fm, _ = split_frontmatter(f.read_text(encoding="utf-8"))
        if fm is None:
            continue
        for src in parse_sources(fm):
            if not ADR_SOURCE_RE.match(src):
                continue
            num = Path(src).name[:4]
            if num in refs and rel(f) not in refs[num]:
                refs[num].append(rel(f))

    mapping: dict[str, str] = {}
    for num in sorted(adrs):
        if num in MANUAL_OVERRIDE:
            mapping[num] = MANUAL_OVERRIDE[num]
            continue
        candidates = sorted(refs[num])
        if len(candidates) == 1:
            mapping[num] = candidates[0]
            continue
        if len(candidates) > 1:
            topical = [c for c in candidates if c not in CROSS_CUTTING]
            if topical:
                mapping[num] = topical[0]
            else:
                mapping[num] = pick_cross_cutting(adrs[num])
            continue
        # 参照ゼロ
        if num in ORPHAN_FALLBACK:
            mapping[num] = ORPHAN_FALLBACK[num]
        else:
            mapping[num] = pick_cross_cutting(adrs[num])
    return mapping, refs


def pick_cross_cutting(path: Path) -> str:
    return (
        "docs/knowledge/product-goals.md"
        if "棄却" in adr_status(path)
        else "docs/knowledge/glossary.md"
    )


ADR_HISTORY_PATH_RE = re.compile(r"^docs/(?:adr|original-docs)/(0\d{3})-.*\.md$")


def _git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=REPO_ROOT, capture_output=True, text=True, check=False
    ).stdout.strip()


def git_dates(adrs: dict[str, Path]) -> dict[str, str]:
    """ADR 番号 → 最初に履歴へ現れた日 (YYYY-MM-DD)。

    素朴な `--diff-filter=A` は使えない。ADR 0073 で `docs/adr/` → `docs/original-docs/` へ
    一括 rename しており、追跡しないと全 ADR が rename 日 (2026-08-09) に潰れる。さらに
    採番のやり直しで slug が変わったものがあるので、パスでなく **ADR 番号**で履歴を舐める。
    それでも拾えない数本（merge commit 経由で tree に現れたもの）はパス指定で最古を取る。
    """
    dates: dict[str, str] = {}

    def offer(num: str, date: str) -> None:
        if date and (num not in dates or date < dates[num]):
            dates[num] = date

    log_out = _git(
        "log", "--all", "--full-history", "--diff-filter=A",
        "--format=%x00%ad", "--date=short", "--name-only",
    )
    commit_date = None
    for line in log_out.split("\n"):
        if line.startswith("\0"):
            commit_date = line[1:].strip()
            continue
        m = ADR_HISTORY_PATH_RE.match(line.strip())
        if m and commit_date:
            offer(m.group(1), commit_date)

    for num, path in adrs.items():
        if num in dates:
            continue
        for p in (rel(path), f"docs/adr/{path.name}"):
            out = _git("log", "--all", "--full-history", "--format=%ad", "--date=short", "--", p)
            if out:
                offer(num, out.split("\n")[-1])
        if num not in dates:
            log(f"  ⚠ {num}: 作成日を特定できず unknown")
            dates[num] = "unknown"
    return dates


def strip_adr_links(text: str) -> str:
    return ADR_LINK_RE.sub(lambda m: m.group(1), text)


def render_entry(num: str, path: Path, date: str) -> str:
    lines = path.read_text(encoding="utf-8").split("\n")
    title = ""
    out: list[str] = []
    in_fence = False
    fence_marker = ""
    skip_status = False

    for line in lines:
        fence = FENCE_RE.match(line)
        if fence:
            marker = fence.group(1)
            if not in_fence:
                in_fence, fence_marker = True, marker
            elif marker == fence_marker:
                in_fence = False
            if not skip_status:
                out.append(line)
            continue

        if not in_fence:
            m = ADR_TITLE_RE.match(line)
            if m and not title:
                title = m.group(2)
                continue
            if re.match(r"^##\s+ステータス\s*$", line):
                skip_status = True
                continue
            if skip_status:
                if re.match(r"^#{1,6}\s", line):
                    skip_status = False
                else:
                    continue
            if re.match(r"^#{2,4}\s", line):
                level = len(line) - len(line.lstrip("#"))
                line = "#" * min(level + 2, 6) + line[level:]

        out.append(line)

    body = strip_adr_links("\n".join(out)).strip("\n")
    status = strip_adr_links(adr_status(path)).strip()
    label = status_label(status)

    heading = f"### ADR {num}: {title} ({date})"
    if label:
        heading += f" — {label}"

    parts = [heading, body]
    if status and status != label:
        # 見出しに載らない但し書き（supersede 先・再検証条件など）を落とさない
        parts.insert(1, f"#### ステータス\n\n{status}")
    return "\n\n".join(parts) + "\n"


def clean_sources(fm_lines: list[str]) -> list[str]:
    block = find_block(fm_lines, "sources")
    if block is None:
        return fm_lines
    start, end = block
    kept: list[str] = []
    for line in fm_lines[start + 1 : end]:
        m = re.match(r"^\s+-\s+(.*?)\s*$", line)
        if m and ADR_SOURCE_RE.match(m.group(1).strip().strip("\"'")):
            continue
        kept.append(line)

    has_item = any(re.match(r"^\s+-\s+", line) for line in kept)
    if has_item:
        return fm_lines[:start] + [fm_lines[start]] + kept + fm_lines[end:]

    # sources が空になったら sources ごと（と distilled_from_sha も）落とす
    result = fm_lines[:start] + fm_lines[end:]
    sha = find_block(result, "distilled_from_sha")
    if sha is not None:
        result = result[: sha[0]] + result[sha[1] :]
    return result


def rewrite_body_links(body: str) -> str:
    out: list[str] = []
    in_fence = False
    fence_marker = ""
    for line in body.split("\n"):
        fence = FENCE_RE.match(line)
        if fence:
            marker = fence.group(1)
            if not in_fence:
                in_fence, fence_marker = True, marker
            elif marker == fence_marker:
                in_fence = False
            out.append(line)
            continue
        out.append(line if in_fence else strip_adr_links(line))
    return "\n".join(out)


def execute(mapping: dict[str, str], adrs: dict[str, Path], dates: dict[str, str]) -> None:
    by_target: dict[str, list[str]] = {}
    for num, target in mapping.items():
        by_target.setdefault(target, []).append(num)

    # Step 4: 決定ログ節を追記
    for target, nums in sorted(by_target.items()):
        path = REPO_ROOT / target
        text = path.read_text(encoding="utf-8")
        if not text.endswith("\n"):
            text += "\n"
        entries = [render_entry(n, adrs[n], dates[n]) for n in sorted(nums)]
        path.write_text(text + DECISION_LOG_HEADER + "\n".join(entries), encoding="utf-8")
        log(f"  決定ログ追記: {target} ({len(nums)} 件)")

    # Step 5/6: sources 掃除 + 本文リンク書き換え（全 knowledge/specifications）
    for f in target_files():
        text = f.read_text(encoding="utf-8")
        fm, body = split_frontmatter(text)
        body = rewrite_body_links(body)
        if fm is None:
            f.write_text(body, encoding="utf-8")
        else:
            f.write_text(join_frontmatter(clean_sources(fm), body), encoding="utf-8")
    log(f"  sources 掃除 + リンク書き換え: {len(target_files())} ファイル")

    # Step 7: ADR 本体を削除
    for num, path in sorted(adrs.items()):
        path.unlink()
    log(f"  ADR 削除: {len(adrs)} ファイル")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--execute", action="store_true", help="実際に移設する")
    ap.add_argument("--dry-run", action="store_true", help="マッピングだけ出す（既定）")
    args = ap.parse_args()

    adrs = adr_files()
    log(f"ADR 検出: {len(adrs)} 本")
    mapping, refs = build_mapping(adrs)
    dates = git_dates(adrs)

    if not args.execute:
        by_target: dict[str, list[str]] = {}
        for num, target in mapping.items():
            by_target.setdefault(target, []).append(num)
        report = {
            "adr_count": len(adrs),
            "mapping": {
                num: {
                    "file": adrs[num].name,
                    "target": mapping[num],
                    "date": dates[num],
                    "referenced_by": refs[num],
                }
                for num in sorted(mapping)
            },
            "by_target": {t: sorted(n) for t, n in sorted(by_target.items())},
            "target_count": len(by_target),
        }
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return 0

    log("移行を実行します")
    execute(mapping, adrs, dates)
    log("完了")
    return 0


if __name__ == "__main__":
    sys.exit(main())
