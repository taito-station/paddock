#!/usr/bin/env python3
"""check-decision-log-immutability.py の回帰テスト（#652）。

本番の docs/ は正常なままなので、検査が壊れても「✓」が出続けて気づけない。使い捨ての
fixture リポジトリを実際に作り、merge-base 比較まで含めた各分岐の終了コードを固定する。

`scripts/test-check-doc-classes.py` と同じ「自走式」（`def test_*()` + assert を末尾の
main() が集めて実行する）。stdlib のみ。

使い方:
  scripts/test-check-decision-log-immutability.py
"""

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

TARGET = Path(__file__).resolve().parent / "check-decision-log-immutability.py"

DOC_TEMPLATE = """---
status: Confirmed
kind: knowledge
doc_class: [D19]
tags: [D19]
sources:
  - docs/original-docs/652-abolish-adr.md
distilled_from_sha: "0000000"
updated: "2026-08-23"
---

# {title}

{body}
"""

LOG_HEADER = (
    "\n---\n\n## 決定ログ\n\n"
    "<!-- この節は append-only です。既存エントリの変更・削除は CI が検出します。 -->\n\n"
)

ENTRY_A = "### 予測モデルは市場ブレンドを既定にする\n\n- 決定: α=0.2 で市場単勝とブレンドする\n- 理由: 純モデルの resolution が天井\n"
ENTRY_B = "### 相手は 3 券種とも top5 にそろえる\n\n- 決定: ワイドも top5\n- 理由: 262R で top3 と有意差なし\n"


def run_git(repo: Path, *args: str) -> str:
    proc = subprocess.run(
        ["git", *args], cwd=repo, capture_output=True, text=True, check=True
    )
    return proc.stdout.strip()


def commit_all(repo: Path, message: str) -> str:
    run_git(repo, "add", "-A")
    run_git(repo, "commit", "-q", "-m", message)
    return run_git(repo, "rev-parse", "HEAD")


def check(repo: Path, *args: str) -> "tuple[int, str]":
    proc = subprocess.run(
        [sys.executable, str(TARGET), *args], cwd=repo, capture_output=True, text=True
    )
    return proc.returncode, proc.stdout + proc.stderr


def write_doc(repo: Path, rel: str, body: str, title: str = "テスト文書") -> None:
    path = repo / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(DOC_TEMPLATE.format(title=title, body=body), encoding="utf-8")


def new_repo() -> Path:
    """main ブランチに 1 文書（決定ログ 1 件）を置いた fixture を作り、作業ブランチへ移る。"""
    repo = Path(tempfile.mkdtemp(prefix="decision-log-test-"))
    run_git(repo, "init", "-q")
    run_git(repo, "config", "user.email", "test@example.invalid")
    run_git(repo, "config", "user.name", "test")
    run_git(repo, "config", "commit.gpgsign", "false")
    (repo / "docs/knowledge").mkdir(parents=True)
    (repo / "docs/specifications").mkdir(parents=True)
    (repo / "docs/original-docs").mkdir(parents=True)
    (repo / "docs/original-docs/652-abolish-adr.md").write_text("# 652\n", encoding="utf-8")
    write_doc(repo, "docs/knowledge/a.md", "本文の段落。\n" + LOG_HEADER + ENTRY_A)
    commit_all(repo, "baseline")
    # `git init -b main` は git 2.28 未満で使えないので、コミット後に改名する。
    run_git(repo, "branch", "-M", "main")
    run_git(repo, "switch", "-q", "-c", "docs/work")
    return repo


def land_on_main(repo: Path) -> None:
    """作業ブランチの現状を main へ取り込み、新しい作業ブランチへ移る。

    「main に載った決定を後から消す/変える」を再現するために要る。比較の基準は
    merge-base なので、同じブランチ内で足して消しただけのエントリは base に存在せず、
    （仕様どおり）落ちない。
    """
    current = run_git(repo, "rev-parse", "--abbrev-ref", "HEAD")
    run_git(repo, "switch", "-q", "main")
    run_git(repo, "merge", "-q", "--ff-only", current)
    run_git(repo, "switch", "-q", "-c", f"{current}-next")


def read_doc(repo: Path, rel: str) -> str:
    return (repo / rel).read_text(encoding="utf-8")


def overwrite(repo: Path, rel: str, text: str) -> None:
    (repo / rel).write_text(text, encoding="utf-8")


# --- 通過するケース ---------------------------------------------------------


def test_untouched_tree_passes() -> None:
    """何も変えていない作業ブランチは通る。"""
    repo = new_repo()
    try:
        code, out = check(repo)
        assert code == 0, out
        assert "✓ 決定ログの不変性を確認" in out, out
        assert "1 本検査" in out, f"検査本数が出ていない:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_new_file_with_decision_log_passes() -> None:
    """base に無い新規ファイルは、決定ログを持っていても全件が新規なので通る。"""
    repo = new_repo()
    try:
        write_doc(repo, "docs/specifications/new.md", "本文。\n" + LOG_HEADER + ENTRY_B)
        commit_all(repo, "新規文書を追加")
        code, out = check(repo)
        assert code == 0, out
        # 新規ファイルは比較対象が無いので検査本数に数えない。
        assert "1 本検査" in out, f"新規ファイルを検査本数に数えている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_appended_entry_passes() -> None:
    """末尾へのエントリ追記は通る（これが append-only の許可された唯一の操作）。"""
    repo = new_repo()
    try:
        overwrite(repo, "docs/knowledge/a.md", read_doc(repo, "docs/knowledge/a.md") + "\n" + ENTRY_B)
        commit_all(repo, "決定ログを追記")
        code, out = check(repo)
        assert code == 0, out
    finally:
        shutil.rmtree(repo)


def test_body_change_outside_decision_log_passes() -> None:
    """決定ログの外（本文）はいくら書き換えても通る。"""
    repo = new_repo()
    try:
        text = read_doc(repo, "docs/knowledge/a.md")
        overwrite(repo, "docs/knowledge/a.md", text.replace("本文の段落。", "本文を全面的に書き直した。"))
        commit_all(repo, "本文を改稿")
        code, out = check(repo)
        assert code == 0, out
    finally:
        shutil.rmtree(repo)


def test_document_without_decision_log_passes() -> None:
    """決定ログ節を持たない文書は検査対象外（base にも無いので比較しない）。"""
    repo = new_repo()
    try:
        write_doc(repo, "docs/knowledge/plain.md", "決定ログの無い文書。\n")
        commit_all(repo, "決定ログ無しの文書を追加")
        overwrite(
            repo,
            "docs/knowledge/plain.md",
            read_doc(repo, "docs/knowledge/plain.md") + "\n追記。\n",
        )
        commit_all(repo, "その文書を書き換える")
        code, out = check(repo)
        assert code == 0, out
        assert "1 本検査" in out, f"決定ログ無しの文書を数えている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_trailing_whitespace_only_change_passes() -> None:
    """行末空白の増減だけでは落とさない（エディタ由来の差分）。"""
    repo = new_repo()
    try:
        text = read_doc(repo, "docs/knowledge/a.md")
        overwrite(repo, "docs/knowledge/a.md", text.replace("- 決定: α=0.2 で市場単勝とブレンドする", "- 決定: α=0.2 で市場単勝とブレンドする   "))
        commit_all(repo, "行末に空白が入った")
        code, out = check(repo)
        assert code == 0, out
    finally:
        shutil.rmtree(repo)


def test_trailing_blank_lines_removed_passes() -> None:
    """節末尾の空行が**減って**も落とさない。

    増える方向は素の prefix 比較でも通ってしまうので、正規化が効いているか判らない。
    base 側に空行を積んでから消す（減る方向）が、末尾空行を落とす処理の唯一の当たり所。
    """
    repo = new_repo()
    try:
        overwrite(repo, "docs/knowledge/a.md", read_doc(repo, "docs/knowledge/a.md") + "\n\n\n")
        commit_all(repo, "末尾に空行を積む")
        land_on_main(repo)
        overwrite(repo, "docs/knowledge/a.md", read_doc(repo, "docs/knowledge/a.md").rstrip() + "\n")
        commit_all(repo, "末尾の空行を消す")
        code, out = check(repo)
        assert code == 0, f"末尾空行の削除で落ちている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_section_after_decision_log_can_be_edited() -> None:
    """決定ログ節の**後ろ**にある別の節は自由に書き換えられる。

    節の終端（次の h2 で打ち切る）が効いているかの唯一の当たり所。節を「足す」だけだと
    素の prefix 比較でも通ってしまうので、base に置いた後続節を**書き換える**。
    """
    repo = new_repo()
    try:
        overwrite(
            repo,
            "docs/knowledge/a.md",
            read_doc(repo, "docs/knowledge/a.md") + "\n## 参考\n\n- リンク集\n",
        )
        commit_all(repo, "参考節を追加")
        land_on_main(repo)
        text = read_doc(repo, "docs/knowledge/a.md")
        overwrite(repo, "docs/knowledge/a.md", text.replace("- リンク集", "- 全面的に書き直したリンク集"))
        commit_all(repo, "参考節を改稿")
        code, out = check(repo)
        assert code == 0, f"決定ログの外の節の編集で落ちている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_decision_log_heading_in_code_fence_is_ignored() -> None:
    """コードフェンス内の `## 決定ログ` は見出しと見なさない（規約文書の見本）。"""
    repo = new_repo()
    try:
        write_doc(
            repo,
            "docs/knowledge/guide.md",
            "書き方の見本:\n\n```markdown\n## 決定ログ\n\n### 見本のエントリ\n```\n",
        )
        commit_all(repo, "規約文書を追加")
        # main へ載せてから触る。新規ファイルのままだと base に無く、比較まで到達しない
        # ＝フェンス判定を壊しても素通りするテストになる。
        land_on_main(repo)
        text = read_doc(repo, "docs/knowledge/guide.md")
        overwrite(repo, "docs/knowledge/guide.md", text.replace("### 見本のエントリ", "### 見本を差し替えた"))
        commit_all(repo, "見本を差し替える")
        code, out = check(repo)
        assert code == 0, out
        assert "1 本検査" in out, f"フェンス内の見出しを節として拾っている:\n{out}"
    finally:
        shutil.rmtree(repo)


# --- 落ちるケース -----------------------------------------------------------


def test_modified_entry_is_error() -> None:
    """既存エントリの本文を書き換えたら落とす。"""
    repo = new_repo()
    try:
        text = read_doc(repo, "docs/knowledge/a.md")
        overwrite(repo, "docs/knowledge/a.md", text.replace("α=0.2", "α=0.5"))
        commit_all(repo, "既存の決定を書き換える")
        code, out = check(repo)
        assert code == 1, f"既存エントリの改変を検出できていない:\n{out}"
        assert "既存エントリが変更されている" in out, out
        assert "docs/knowledge/a.md" in out, out
    finally:
        shutil.rmtree(repo)


def test_modified_entry_heading_is_error() -> None:
    """見出しは同じで本文だけ変えても落とす（見出し一致で素通りさせない）。"""
    repo = new_repo()
    try:
        text = read_doc(repo, "docs/knowledge/a.md")
        overwrite(
            repo,
            "docs/knowledge/a.md",
            text.replace("- 理由: 純モデルの resolution が天井", "- 理由: あとから書き換えた理由"),
        )
        commit_all(repo, "理由だけ差し替える")
        code, out = check(repo)
        assert code == 1, f"エントリ本文の差し替えを検出できていない:\n{out}"
        assert "既存エントリが変更されている" in out, out
    finally:
        shutil.rmtree(repo)


def test_deleted_entry_is_error() -> None:
    """既存エントリを消したら落とす。

    削除するエントリは **main に取り込んでから**消す。比較の基準は merge-base なので、
    同じブランチ内で足して消したエントリは base に存在せず（正しく）通ってしまう。
    """
    repo = new_repo()
    try:
        write_doc(repo, "docs/knowledge/a.md", "本文の段落。\n" + LOG_HEADER + ENTRY_A + "\n" + ENTRY_B)
        commit_all(repo, "2 件目を追記")
        land_on_main(repo)
        write_doc(repo, "docs/knowledge/a.md", "本文の段落。\n" + LOG_HEADER + ENTRY_A)
        commit_all(repo, "2 件目を削除")
        code, out = check(repo)
        assert code == 1, f"エントリ削除を検出できていない:\n{out}"
        assert "既存エントリが削除されている" in out, out
    finally:
        shutil.rmtree(repo)


def test_deleted_section_is_error() -> None:
    """決定ログ節ごと消したら落とす。"""
    repo = new_repo()
    try:
        write_doc(repo, "docs/knowledge/a.md", "本文の段落。\n")
        commit_all(repo, "決定ログ節を削除")
        code, out = check(repo)
        assert code == 1, f"節の削除を検出できていない:\n{out}"
        assert "決定ログ節ごと消えている" in out, out
    finally:
        shutil.rmtree(repo)


def test_renamed_heading_is_error() -> None:
    """見出しを別文言へ改名したら「節ごと消えた」として落とす。"""
    repo = new_repo()
    try:
        text = read_doc(repo, "docs/knowledge/a.md")
        overwrite(repo, "docs/knowledge/a.md", text.replace("## 決定ログ", "## 決定の記録"))
        commit_all(repo, "見出しを改名")
        code, out = check(repo)
        assert code == 1, f"見出しの改名を検出できていない:\n{out}"
        assert "決定ログ節ごと消えている" in out, out
    finally:
        shutil.rmtree(repo)


def test_inserted_entry_at_head_is_error() -> None:
    """既存エントリの**前**への挿入は追記ではないので落とす。"""
    repo = new_repo()
    try:
        write_doc(repo, "docs/knowledge/a.md", "本文の段落。\n" + LOG_HEADER + ENTRY_B + "\n" + ENTRY_A)
        commit_all(repo, "先頭へ挿入")
        code, out = check(repo)
        assert code == 1, f"先頭への挿入を検出できていない:\n{out}"
        assert "既存エントリが変更されている" in out, out
    finally:
        shutil.rmtree(repo)


def test_specifications_dir_is_also_checked() -> None:
    """docs/specifications 側も同じ規律で見る（対象ディレクトリの取りこぼし防止）。"""
    repo = new_repo()
    try:
        write_doc(repo, "docs/specifications/s.md", "仕様。\n" + LOG_HEADER + ENTRY_A)
        commit_all(repo, "仕様書を追加")
        land_on_main(repo)
        text = read_doc(repo, "docs/specifications/s.md")
        overwrite(repo, "docs/specifications/s.md", text.replace("α=0.2", "α=0.9"))
        commit_all(repo, "仕様書の決定ログを改変")
        code, out = check(repo)
        assert code == 1, f"specifications 側を見ていない:\n{out}"
        assert "docs/specifications/s.md" in out, out
    finally:
        shutil.rmtree(repo)


def test_violation_reports_both_sides() -> None:
    """error 行に base と現在の両方が出る（何が変わったか読める）。"""
    repo = new_repo()
    try:
        text = read_doc(repo, "docs/knowledge/a.md")
        overwrite(repo, "docs/knowledge/a.md", text.replace("α=0.2", "α=0.5"))
        commit_all(repo, "既存の決定を書き換える")
        code, out = check(repo)
        assert code == 1, out
        assert "- base:" in out and "+ 現在:" in out, f"差分の両側が出ていない:\n{out}"
        assert "α=0.2" in out and "α=0.5" in out, out
    finally:
        shutil.rmtree(repo)


def main() -> int:
    if not TARGET.is_file():
        print(f"テスト対象が見つからない: {TARGET}", file=sys.stderr)
        return 1
    tests = [(n, f) for n, f in sorted(globals().items()) if n.startswith("test_") and callable(f)]
    failures = 0
    print("check-decision-log-immutability.py 回帰テスト")
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
