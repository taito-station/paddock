#!/usr/bin/env python3
"""bump-distilled-sha.py の回帰テスト（#604）。

この道具は **docs の frontmatter を書き換える**。壊れたときの被害が「規約の正本を黙って
書き換える」なので、fail-open と誤書き換えの経路を固定する。とくに:

  - checker が STALE 以外の理由で落ちたとき「直すものは無い」と言わない
  - frontmatter を持たない文書（README）の本文テンプレートを書き換えない
  - 途中で失敗したとき、先行ファイルだけ書き換わった半端な状態を残さない

fixture 生成は `test-check-doc-classes.py` の関数を再利用する（同じ形の使い捨て git repo）。
自走式（`def test_*()` を末尾の main() が集める）・stdlib のみ。

使い方:
  scripts/test-bump-distilled-sha.py
"""

import importlib.util
import re
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
TARGET = HERE / "bump-distilled-sha.py"

_spec = importlib.util.spec_from_file_location("dc_test", HERE / "test-check-doc-classes.py")
if _spec is None or _spec.loader is None:  # pragma: no cover
    sys.exit("test-check-doc-classes.py を読み込めない")
_dc = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_dc)

new_repo = _dc.new_repo
baseline = _dc.baseline
write_doc = _dc.write_doc
write_registry = _dc.write_registry
commit_all = _dc.commit_all


def run(repo: Path, *args: str) -> "tuple[int, str]":
    proc = subprocess.run(
        [sys.executable, str(TARGET), *args], cwd=repo, capture_output=True, text=True
    )
    return proc.returncode, proc.stdout + proc.stderr


def distilled_of(repo: Path, rel: str) -> str:
    for line in (repo / rel).read_text(encoding="utf-8").splitlines():
        if line.startswith("distilled_from_sha:"):
            return line.split(":", 1)[1].strip().strip('"')
    return ""


def make_stale(repo: Path) -> str:
    """a.md の source（0001-first.md）を本文ごと更新して STALE を作る。"""
    (repo / "docs/original-docs/0001-first.md").write_text(
        "# 0001. 最初の決定\n\n## 決定\n\n本文を変えた。\n", encoding="utf-8"
    )
    return commit_all(repo, "source の本文を変更")


def test_all_stale_bumps_target() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        changed = make_stale(repo)
        code, out = run(repo, "--all-stale")
        assert code == 0, out
        assert "docs/knowledge/a.md" in out, out
        assert distilled_of(repo, "docs/knowledge/a.md") == changed, out
    finally:
        shutil.rmtree(repo)


def test_dry_run_does_not_write() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        before = distilled_of(repo, "docs/knowledge/a.md")
        make_stale(repo)
        code, out = run(repo, "--all-stale", "--dry-run")
        assert code == 0, out
        assert "（dry-run）" in out, out
        assert distilled_of(repo, "docs/knowledge/a.md") == before, "dry-run で書き換わった"
    finally:
        shutil.rmtree(repo)


def test_no_stale_reports_nothing_to_do() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        code, out = run(repo, "--all-stale")
        assert code == 0, out
        assert "STALE な文書は無い" in out, out
    finally:
        shutil.rmtree(repo)


def test_checker_failure_is_not_reported_as_no_stale() -> None:
    """checker が別の理由で落ちているのに「直すものは無い」と言わない（fail-open 潰し）。"""
    repo = new_repo()
    try:
        baseline(repo)
        before = distilled_of(repo, "docs/knowledge/a.md")
        registry = repo / "docs/knowledge/doc-classes.md"
        registry.write_text(
            registry.read_text(encoding="utf-8").replace("<!-- doc-classes-index:begin -->", ""),
            encoding="utf-8",
        )
        code, out = run(repo, "--all-stale")
        assert code != 0, out
        assert "STALE 以外の理由で落ちている" in out, out
        assert distilled_of(repo, "docs/knowledge/a.md") == before, "落ちたのに書き換えた"
    finally:
        shutil.rmtree(repo)


def test_frontmatter_only_is_rewritten() -> None:
    """本文（フェンス内のテンプレート例）は書き換えない。"""
    repo = new_repo()
    try:
        baseline(repo)
        path = repo / "docs/knowledge/a.md"
        path.write_text(
            path.read_text(encoding="utf-8")
            + '\n```yaml\ndistilled_from_sha: "<short-sha>"\n```\n',
            encoding="utf-8",
        )
        head = commit_all(repo, "テンプレ例を足す")
        code, out = run(repo, "docs/knowledge/a.md")
        assert code == 0, out
        assert distilled_of(repo, "docs/knowledge/a.md") == head, out
        assert '"<short-sha>"' in path.read_text(encoding="utf-8"), "本文のテンプレを書き換えた"
    finally:
        shutil.rmtree(repo)


def test_file_without_frontmatter_is_refused() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        (repo / "docs/knowledge/plain.md").write_text(
            '# 規約\n\n```yaml\ndistilled_from_sha: "<short-sha>"\n```\n', encoding="utf-8"
        )
        code, out = run(repo, "docs/knowledge/plain.md")
        assert code == 1, out
        assert "distilled_from_sha の行が無い" in out, out
        assert '"<short-sha>"' in (repo / "docs/knowledge/plain.md").read_text(encoding="utf-8")
    finally:
        shutil.rmtree(repo)


def test_missing_file_aborts_before_writing() -> None:
    """途中で落ちるとき、先行ファイルだけ書き換わった半端な状態を残さない。"""
    repo = new_repo()
    try:
        baseline(repo)
        before = distilled_of(repo, "docs/knowledge/a.md")
        code, out = run(repo, "docs/knowledge/a.md", "docs/knowledge/nope.md")
        assert code == 1, out
        assert "ファイルが無い" in out, out
        assert distilled_of(repo, "docs/knowledge/a.md") == before, "abort 前に書き換えた"
    finally:
        shutil.rmtree(repo)


def test_sha_option_requires_a_value() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        before = distilled_of(repo, "docs/knowledge/a.md")
        code, out = run(repo, "--sha", "--dry-run", "docs/knowledge/a.md")
        assert code == 2, out
        assert "--sha に値が無い" in out, out
        assert distilled_of(repo, "docs/knowledge/a.md") == before, out
    finally:
        shutil.rmtree(repo)


def test_unresolvable_sha_is_rejected() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        before = distilled_of(repo, "docs/knowledge/a.md")
        code, out = run(repo, "--sha", "zzzzzzz", "docs/knowledge/a.md")
        assert code == 2, out
        assert "解決できない" in out, out
        assert distilled_of(repo, "docs/knowledge/a.md") == before, out
    finally:
        shutil.rmtree(repo)


def test_updated_is_not_touched() -> None:
    """`updated` は人が判断して進める。道具が勝手に動かさない。"""
    repo = new_repo()
    try:
        baseline(repo)
        before = (repo / "docs/knowledge/a.md").read_text(encoding="utf-8")
        make_stale(repo)
        code, out = run(repo, "--all-stale")
        assert code == 0, out
        after = (repo / "docs/knowledge/a.md").read_text(encoding="utf-8")
        assert 'updated: "2026-08-09"' in after, after
        assert before != after, "sha が変わっていない"
    finally:
        shutil.rmtree(repo)


def test_sha_option_writes_resolved_sha() -> None:
    """`--sha HEAD` を literal で書くと、その文書の stale 判定が恒久的に無効化される。"""
    repo = new_repo()
    try:
        baseline(repo)
        code, out = run(repo, "--sha", "HEAD", "docs/knowledge/a.md")
        assert code == 0, out
        written = distilled_of(repo, "docs/knowledge/a.md")
        assert written != "HEAD", f"可変参照をそのまま書いた: {written}"
        assert len(written) >= 7, written
    finally:
        shutil.rmtree(repo)


def test_duplicate_distilled_lines_are_refused() -> None:
    """checker は最後の行、bump は最初の行を見る。放置すると bump しても STALE が消えない。"""
    repo = new_repo()
    try:
        baseline(repo)
        path = repo / "docs/knowledge/a.md"
        text = path.read_text(encoding="utf-8")
        dup = 'distilled_from_sha: "deadbee"\nupdated:'
        path.write_text(text.replace("updated:", dup, 1), encoding="utf-8")
        code, out = run(repo, "docs/knowledge/a.md")
        assert code == 1, out
        assert "distilled_from_sha が 2 行ある" in out, out
    finally:
        shutil.rmtree(repo)


def test_no_args_is_fail_closed() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        code, out = run(repo)
        assert code == 2, out
        assert "引数が無い" in out, out
    finally:
        shutil.rmtree(repo)


def test_other_errors_are_not_reported_as_resolved() -> None:
    """STALE と他の error が同居するとき、bump 後に 0 を返して「解消した」と読ませない。"""
    repo = new_repo()
    try:
        baseline(repo)
        make_stale(repo)
        write_doc(repo, "docs/knowledge/a.md", ["D19"],
                  ["docs/original-docs/9999-nope.md", "docs/original-docs/0001-first.md"],
                  distilled_of(repo, "docs/knowledge/a.md"))
        commit_all(repo, "存在しない source を足す（別の error）")
        code, out = run(repo, "--all-stale")
        assert code == 1, out
        assert "STALE 以外の error も残っている" in out, out
    finally:
        shutil.rmtree(repo)


def test_body_template_after_frontmatter_without_sha_is_refused() -> None:
    """frontmatter に sha 行が無く、本文のフェンス内にだけ見本がある場合を弾く。"""
    repo = new_repo()
    try:
        baseline(repo)
        (repo / "docs/knowledge/tmpl.md").write_text(
            '---\nstatus: Confirmed\nkind: knowledge\n---\n\n'
            '# テンプレ\n\n```yaml\ndistilled_from_sha: "<short-sha>"\n```\n',
            encoding="utf-8",
        )
        code, out = run(repo, "docs/knowledge/tmpl.md")
        assert code == 1, out
        assert "distilled_from_sha の行が無い" in out, out
        body = (repo / "docs/knowledge/tmpl.md").read_text(encoding="utf-8")
        assert '"<short-sha>"' in body, "本文のテンプレを書き換えた"
    finally:
        shutil.rmtree(repo)


def test_crlf_document_is_bumped_in_place() -> None:
    """CRLF の文書でも frontmatter を見つけ、改行コードを壊さない。"""
    repo = new_repo()
    try:
        baseline(repo)
        path = repo / "docs/knowledge/a.md"
        with path.open(encoding="utf-8", newline="") as f:
            lf_text = f.read()
        with path.open("w", encoding="utf-8", newline="") as f:
            f.write(lf_text.replace("\n", "\r\n"))
        head = commit_all(repo, "CRLF へ変換")
        code, out = run(repo, "docs/knowledge/a.md")
        assert code == 0, out
        with path.open(encoding="utf-8", newline="") as f:
            after = f.read()
        assert f'distilled_from_sha: "{head}"' in after, after[:200]
        assert after.count("\n") == after.count("\r\n") > 0, "改行コードが LF へ正規化された"
    finally:
        shutil.rmtree(repo)


def test_empty_value_gets_a_space_after_colon() -> None:
    """値が空の行を bump しても YAML として壊れた `key:"v"` を書かない。"""
    repo = new_repo()
    try:
        baseline(repo)
        path = repo / "docs/knowledge/a.md"
        text = path.read_text(encoding="utf-8")
        path.write_text(
            re.sub(r'^distilled_from_sha: ".*"$', "distilled_from_sha:", text, count=1, flags=re.M),
            encoding="utf-8",
        )
        code, out = run(repo, "docs/knowledge/a.md")
        assert code == 0, out
        after = path.read_text(encoding="utf-8")
        assert 'distilled_from_sha: "' in after, after[:200]
        assert 'distilled_from_sha:"' not in after, "コロン直後に空白が無い"
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
