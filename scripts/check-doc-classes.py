#!/usr/bin/env python3
"""knowledge/specifications の frontmatter を機械検査する（ADR 0073 決定 2・3）。

ADR 0073 で「ADR の内容は knowledge へ全部写す」を選んだ。重複を許す代わりに、同期切れを
機械で検出するのが本スクリプト。人手の規律だけでは守れないことは実証済みで、
docs/knowledge/app-bootstrap.md が status: Confirmed のまま存在しない NoopParser を推奨し
続けていた（qa 側には「#453 で覆る」と追記済みだった。#578 で解消）。

検査項目:
  1. doc_class が docs/knowledge/doc-classes.md の定義済みクラスか            [error]
  2. doc_class に n/a 宣言済みクラスが含まれていないか                       [error]
  3. tags が doc_class と完全一致するか（値も順序も）                        [error]
  4. sources に列挙されたパスが実在するか                                     [error]
  5. doc-classes.md の「現行」列が実態と一致するか                            [error]
  6. stale: sources の最終「内容変更」が distilled_from_sha に含まれているか  [warning]
  7. active なのに文書 0 本のクラス（充足ギャップ）                           [warning]

6 は rename-only のコミット（内容差分ゼロ）を比較対象から除外する。これが無いと ADR 0073 の
ADR 移動だけで 20 本が一斉に stale 判定になる。git log --follow では吸収できない——--follow は
リネームより前へ履歴を遡らせるだけで、「最終コミット」がリネームコミットになる事実は変わらない。

依存は標準ライブラリのみ（PyYAML を使わない）。CI の predict-check ジョブと同じ前提で、
frontmatter は限定的な構造しか取らないため正規表現で足りる。

使い方:
  scripts/check-doc-classes.py               # 検査（error があれば非ゼロ終了）
  scripts/check-doc-classes.py check         # 同上
  scripts/check-doc-classes.py --warn-only   # error も警告として報告し常に 0 で終了
"""

import re
import subprocess
import sys
from pathlib import Path

USAGE = """check-doc-classes.py - 文書クラスと sources 追従の機械検査（ADR 0073）

使い方:
  scripts/check-doc-classes.py               # 検査（error があれば非ゼロ終了）
  scripts/check-doc-classes.py check         # 同上
  scripts/check-doc-classes.py --warn-only   # error も警告扱いにして常に 0 で終了

オプション:
  -h, --help   このヘルプ
"""

# クラス定義の正本。この 1 ファイルだけが「どのクラスが存在するか」を決める。
REGISTRY = Path("docs/knowledge/doc-classes.md")

# 検査対象のディレクトリ。両方とも「その場で knowledge」として frontmatter を持つ。
TARGET_DIRS = ("docs/knowledge", "docs/specifications")

# 走査から外すファイル。
#   README.md    : 規約そのもの。frontmatter のテンプレート例（0NNN-....md 等の
#                  存在しないパス）を含むため、走査すると必ず偽陽性になる。
#   doc-classes.md: クラス定義そのもの。doc_class は持たない（sources/stale だけ検査する）。
EXCLUDED_FROM_DOC_CLASS = {"README.md", "doc-classes.md"}
EXCLUDED_ENTIRELY = {"README.md"}

# frontmatter は限定的な構造しか取らないので正規表現で読む。書式は doc-classes.md が規定。
RE_FLOW_LIST = re.compile(r"^(doc_class|tags):\s*\[([^\]]*)\]\s*$")
RE_SOURCES_HEAD = re.compile(r"^sources:\s*$")
RE_SOURCES_ITEM = re.compile(r"^\s+-\s+(\S+)")
RE_SCALAR = re.compile(r'^(status|kind|distilled_from_sha|updated):\s*"?([^"#]*?)"?\s*(?:#.*)?$')
# レジストリの表。行数や見出し文言に依存しないようマーカーで範囲を切り出す。
RE_CLASS_ROW = re.compile(r"^\|\s*(D\d{2})\s*\|([^|]*)\|\s*(active|n/a)\s*\|\s*(\d+)\s*\|")
RE_NA_ROW = re.compile(r"^\|\s*(D\d{2})\s*\|")


def git(*args: str) -> "subprocess.CompletedProcess[str]":
    return subprocess.run(["git", *args], capture_output=True, text=True)


def repo_root() -> Path:
    proc = git("rev-parse", "--show-toplevel")
    if proc.returncode != 0:
        sys.exit("git リポジトリ外では実行できない")
    return Path(proc.stdout.strip())


def extract_block(text: str, name: str) -> list[str]:
    """`<!-- name:begin -->` … `<!-- name:end -->` に挟まれた行を返す。"""
    begin, end = f"<!-- {name}:begin -->", f"<!-- {name}:end -->"
    if begin not in text or end not in text:
        sys.exit(f"{REGISTRY} に {begin} / {end} のマーカーが無い（表の範囲を切り出せない）")
    return text.split(begin, 1)[1].split(end, 1)[0].splitlines()


def parse_frontmatter(path: Path) -> dict:
    """frontmatter を dict で返す。無ければ空 dict。

    `---` 直後の `#` 始まり行（specifications が持つ YAML コメント）は読み飛ばす。
    """
    lines = path.read_text(encoding="utf-8").splitlines()
    if not lines or lines[0].strip() != "---":
        return {}
    out: dict = {}
    in_sources = False
    for line in lines[1:]:
        if line.strip() == "---":
            break
        if line.lstrip().startswith("#"):
            continue
        if in_sources:
            item = RE_SOURCES_ITEM.match(line)
            if item:
                out.setdefault("sources", []).append(item.group(1))
                continue
            in_sources = False
        if RE_SOURCES_HEAD.match(line):
            in_sources = True
            out.setdefault("sources", [])
            continue
        flow = RE_FLOW_LIST.match(line)
        if flow:
            body = flow.group(2).strip()
            out[flow.group(1)] = [v.strip() for v in body.split(",") if v.strip()]
            continue
        scalar = RE_SCALAR.match(line)
        if scalar:
            out[scalar.group(1)] = scalar.group(2).strip()
    return out


def last_content_change(path: str, limit: int = 30) -> "str | None":
    """path の内容が最後に変わったコミットの SHA。rename-only のコミットは飛ばす。

    ディレクトリ移設（ADR 0073 の ADR 移動など）で sources のパスだけが変わった場合、
    その rename コミットを「最終コミット」と見なすと全件が stale になる。内容が同一なら
    その knowledge が反映するリポジトリ状態は変わっていないので、遡って実質の変更点を探す。
    """
    proc = git("log", f"--max-count={limit}", "--format=%H", "--follow", "--", path)
    if proc.returncode != 0:
        return None
    for sha in proc.stdout.split():
        # name-status に **パス指定を渡さない**。渡すとリネーム元が絞り込みから外れて対に
        # ならず、R100 ではなく A（新規追加）として報告される（実測）。コミット全体の
        # name-status を取り、対象パスを終点とする行だけを見る。
        shown = git("show", "--format=", "--name-status", "-M100%", sha).stdout
        status = None
        for line in shown.splitlines():
            parts = line.split("\t")
            if len(parts) >= 2 and parts[-1] == path:
                status = parts[0]
                break
        if status == "R100":
            continue  # 純粋なリネーム。内容は変わっていないので更に遡る
        return sha
    return None


def main(argv: list[str]) -> int:
    if len(argv) > 1:
        print("引数が多すぎる（受理は check / --warn-only / -h のいずれか 1 つ）", file=sys.stderr)
        print(USAGE, file=sys.stderr)
        return 2
    arg = argv[0] if argv else "check"
    if arg in ("-h", "--help"):
        print(USAGE)
        return 0
    if arg not in ("check", "--warn-only"):
        print(f"不明な引数: {arg}", file=sys.stderr)
        print(USAGE, file=sys.stderr)
        return 2
    warn_only = arg == "--warn-only"

    root = repo_root()
    registry_path = root / REGISTRY
    if not registry_path.is_file():
        print(f"クラス定義が見つからない: {REGISTRY}", file=sys.stderr)
        return 1

    errors: list[str] = []
    warnings: list[str] = []

    # --- レジストリを読む ---
    registry_text = registry_path.read_text(encoding="utf-8")
    declared: dict[str, dict] = {}
    for line in extract_block(registry_text, "doc-classes"):
        row = RE_CLASS_ROW.match(line.strip())
        if row:
            declared[row.group(1)] = {"state": row.group(3), "count": int(row.group(4))}
    na_declared = {
        m.group(1)
        for line in extract_block(registry_text, "doc-classes-na")
        if (m := RE_NA_ROW.match(line.strip()))
    }
    if not declared:
        print(f"{REGISTRY} からクラス定義を 1 件も読めなかった（表の書式を確認する）", file=sys.stderr)
        return 1

    # 一覧の n/a と N/A 宣言表が食い違っていないか（片方だけ直す事故を防ぐ）。
    na_in_table = {c for c, meta in declared.items() if meta["state"] == "n/a"}
    for cls in sorted(na_in_table - na_declared):
        errors.append(f"{REGISTRY}: {cls} は一覧で n/a だが N/A 宣言表に理由が無い")
    for cls in sorted(na_declared - na_in_table):
        errors.append(f"{REGISTRY}: {cls} は N/A 宣言表にあるが一覧の状態が n/a になっていない")

    # --- 対象ファイルを走査 ---
    targets: list[Path] = []
    for d in TARGET_DIRS:
        targets.extend(sorted(p for p in (root / d).glob("*.md") if p.name not in EXCLUDED_ENTIRELY))

    actual_count: dict[str, int] = {cls: 0 for cls in declared}
    for path in targets:
        rel = path.relative_to(root).as_posix()
        fm = parse_frontmatter(path)
        if not fm:
            errors.append(f"{rel}: frontmatter が無い")
            continue

        # (1)(2)(3) doc_class / tags
        if path.name not in EXCLUDED_FROM_DOC_CLASS:
            classes = fm.get("doc_class")
            if not classes:
                errors.append(f"{rel}: doc_class が無い（書式は {REGISTRY} 参照）")
            else:
                for cls in classes:
                    if cls not in declared:
                        errors.append(f"{rel}: 未定義のクラス {cls}")
                    elif declared[cls]["state"] == "n/a":
                        errors.append(f"{rel}: N/A 宣言済みのクラス {cls} が指定されている")
                    else:
                        actual_count[cls] += 1
                if fm.get("tags") != classes:
                    errors.append(
                        f"{rel}: tags が doc_class と一致しない"
                        f"（doc_class={classes} / tags={fm.get('tags')}）"
                    )

        # (4) sources の実在
        sources = fm.get("sources", [])
        if not sources:
            errors.append(f"{rel}: sources が空（由来を辿れない）")
        for src in sources:
            if not (root / src).is_file():
                errors.append(f"{rel}: sources のパスが実在しない → {src}")

        # (6) stale
        distilled = fm.get("distilled_from_sha", "")
        if not distilled:
            errors.append(f"{rel}: distilled_from_sha が無い")
            continue
        resolved = git("rev-parse", "--verify", "--quiet", f"{distilled}^{{commit}}")
        if resolved.returncode != 0:
            warnings.append(
                f"{rel}: distilled_from_sha '{distilled}' を解決できない"
                "（shallow clone か、rebase で失われた SHA の可能性）"
            )
            continue
        distilled_full = resolved.stdout.strip()
        for src in sources:
            if not (root / src).is_file():
                continue  # 実在チェックで既に error にしている
            changed = last_content_change(src)
            if changed is None:
                continue
            if git("merge-base", "--is-ancestor", changed, distilled_full).returncode != 0:
                warnings.append(
                    f"{rel}: STALE ← {src} が distilled_from_sha({distilled}) より後に更新されている"
                    f"（{changed[:7]}）。差分マージして sha/日付を更新する"
                )

    # (5) レジストリの「現行」列と実態
    for cls, meta in sorted(declared.items()):
        if meta["count"] != actual_count[cls]:
            errors.append(
                f"{REGISTRY}: {cls} の現行列が実態と合わない"
                f"（表={meta['count']} / 実際={actual_count[cls]}）"
            )
        # (7) 充足ギャップ
        if meta["state"] == "active" and actual_count[cls] == 0:
            warnings.append(f"{cls} は active だが該当文書が 0 本（充足ギャップ）")

    # --- 報告 ---
    for w in warnings:
        print(f"警告: {w}", file=sys.stderr)
    if errors:
        print("", file=sys.stderr)
        for e in errors:
            print(f"✗ {e}", file=sys.stderr)
        print("", file=sys.stderr)
        print(f"✗ {len(errors)} 件の不整合（警告 {len(warnings)} 件）", file=sys.stderr)
        if warn_only:
            print("  --warn-only のため 0 で終了する", file=sys.stderr)
            return 0
        return 1

    print(
        f"✓ 文書クラス・sources 整合を確認（{len(targets)} 本 / 警告 {len(warnings)} 件）"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
