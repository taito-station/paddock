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

import contextlib
import importlib.util
import io
import os
import re
import shutil
import subprocess
import sys
import tempfile
import types
from pathlib import Path

TARGET = Path(__file__).resolve().parent / "check-doc-classes.py"

REGISTRY_TEMPLATE = """---
status: Confirmed
kind: knowledge
sources:
{sources}
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

## 割当の一覧

<!-- doc-classes-index:begin -->
| 文書 | doc_class |
|---|---|
{index}
<!-- doc-classes-index:end -->
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
    # 改行コードを git に触らせない。global 設定が autocrlf=true/input の環境だと add 時に
    # 正規化され、CRLF のケースが「nothing to commit」で例外になってスイートごと落ちる。
    # `core.attributesFile` に `* text=auto` を置いている環境もあるので .gitattributes で
    # 上書きする（リポジトリ内の指定が global の attributes より強い）。
    run_git(repo, "config", "core.autocrlf", "false")
    run_git(repo, "config", "core.safecrlf", "false")
    (repo / ".gitattributes").write_text("* -text\n", encoding="utf-8")
    (repo / "docs/knowledge").mkdir(parents=True)
    (repo / "docs/specifications").mkdir(parents=True)
    (repo / "docs/original-docs").mkdir(parents=True)
    (repo / "docs/original-docs/0001-first.md").write_text(
        "# 0001. 最初の決定\n\n## ステータス\n\n承認済み。\n\n## 決定\n\nこうする。\n",
        encoding="utf-8",
    )
    return repo


def write_registry(
    repo: Path,
    sha: str,
    d08: int = 0,
    d19: int = 1,
    d22: int = 0,
    docs: "list[tuple[str, list[str]]] | None" = None,
    sources: "list[str] | None" = None,
) -> None:
    """レジストリを書く。`docs` は割当索引の行（既定は baseline の 1 本）。

    索引は checker が実ファイルと 1 対 1 で突き合わせるので、文書を足すテストは
    ここにも行を足す必要がある。
    """
    rows = docs if docs is not None else [("knowledge/a.md", ["D19"])]
    index = "\n".join(f"| {rel} | [{', '.join(classes)}] |" for rel, classes in rows)
    src_lines = "\n".join(f"  - {s}" for s in (sources or ["docs/original-docs/0001-first.md"]))
    (repo / "docs/knowledge/doc-classes.md").write_text(
        REGISTRY_TEMPLATE.format(
            sha=sha, d08=d08, d19=d19, d22=d22, index=index, sources=src_lines
        ),
        encoding="utf-8",
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
        write_registry(repo, sha, d19=0, docs=[("knowledge/a.md", ["D99"])])
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
        write_registry(repo, sha, d19=0, docs=[("knowledge/a.md", ["D12"])])
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
        write_registry(repo, sha, d19=1, d22=1, docs=[("knowledge/a.md", ["D19", "D22"])])
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


def test_stale_source_is_error() -> None:
    """source が distilled より後に内容変更されたら stale として **error**（#580 で warning から昇格）。

    「ADR の内容は knowledge へ全部写す」（ADR 0073 決定 2）の担保はこの検査だけなので、
    warning のままだと写した量に比例して追従漏れが静かに溜まる。
    """
    repo = new_repo()
    try:
        baseline(repo)
        p = repo / "docs/original-docs/0001-first.md"
        p.write_text(p.read_text(encoding="utf-8") + "\n追記。\n", encoding="utf-8")
        commit_all(repo, "source を実質更新")
        code, out = check(repo)
        assert code == 1, f"stale は error のはず: {out}"
        assert "STALE" in out, out
    finally:
        shutil.rmtree(repo)


def test_stale_is_suppressed_by_warn_only() -> None:
    """`--warn-only` なら stale でも 0 で終える（既存の逃げ道が効くことを固定する）。"""
    repo = new_repo()
    try:
        baseline(repo)
        p = repo / "docs/original-docs/0001-first.md"
        p.write_text(p.read_text(encoding="utf-8") + "\n追記。\n", encoding="utf-8")
        commit_all(repo, "source を実質更新")
        code, out = check(repo, "--warn-only")
        assert code == 0, out
        assert "STALE" in out, out
    finally:
        shutil.rmtree(repo)


def test_untracked_source_is_warning_not_silent() -> None:
    """履歴を辿れない source は判定不能。黙って通す（fail-open）のではなく可視化する。

    stale を error へ昇格させた（#580）あとは、この warning 分岐が数少ない
    「検査が効かないまま緑になる」経路なので、消えていないことを固定する。
    """
    repo = new_repo()
    try:
        sha = baseline(repo)
        # コミットしない source を sources に足す（git log が空 → last_content_change が None）
        (repo / "docs/original-docs/9999-untracked.md").write_text("# 未コミット\n", encoding="utf-8")
        write_doc(repo, "docs/knowledge/a.md", ["D19"],
                  ["docs/original-docs/0001-first.md", "docs/original-docs/9999-untracked.md"], sha)
        code, out = check(repo)
        assert code == 0, f"判定不能は error にしない: {out}"
        assert "履歴が無く" in out, out
    finally:
        shutil.rmtree(repo)


def test_shallow_clone_downgrades_unresolvable_sha_to_warning() -> None:
    """shallow clone では sha 未解決を warning に落とす（履歴が無いだけなので）。

    ただしその文書の stale 判定は丸ごと飛ぶ。CI は fetch-depth: 0 でこの経路に入らない。
    """
    repo = new_repo()
    shallow = Path(tempfile.mkdtemp(prefix="doc-classes-shallow-"))
    try:
        baseline(repo)
        # 追加コミットを重ねてから深さ 1 で clone すると、pin した sha が clone 側に存在しない
        p = repo / "docs/original-docs/0001-first.md"
        p.write_text(p.read_text(encoding="utf-8") + "\n追記。\n", encoding="utf-8")
        commit_all(repo, "2 つ目のコミット")
        dest = shallow / "clone"
        subprocess.run(["git", "clone", "-q", "--depth", "1", f"file://{repo}", str(dest)],
                       capture_output=True, text=True, check=True)
        assert run_git(dest, "rev-parse", "--is-shallow-repository") == "true"
        code, out = check(dest)
        assert code == 0, f"shallow では error にしない: {out}"
        assert "shallow clone のため" in out, out
    finally:
        shutil.rmtree(repo)
        shutil.rmtree(shallow, ignore_errors=True)


def test_subdirectory_md_is_error() -> None:
    """サブディレクトリの `.md` は**完全に無検査**なので error（ADR 0083 で warning から昇格）。

    glob が非再帰なので、`docs/knowledge/sub/x.md` は doc_class も sources も stale も
    一切検査されない。文書を 1 階層下げるだけで検査域から丸ごと外せてしまう。
    #580 が stale を warning → error に上げたのと同じ理由。
    """
    repo = new_repo()
    try:
        baseline(repo)
        (repo / "docs/knowledge/sub").mkdir()
        (repo / "docs/knowledge/sub/x.md").write_text("# 検査対象外\n", encoding="utf-8")
        code, out = check(repo)
        assert code == 1, f"サブディレクトリ配置を通した: {out}"
        assert "サブディレクトリの .md は検査対象外" in out, out
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
    """active なのに 0 本のクラスは充足ギャップとして警告（error にはしない）。

    n/a のクラス（D12）は 0 本でも警告しない。ここを `or` で繋ぐと第 1 項が常に真になって
    第 2 項が評価されず、判定を `state` 抜きに変異させてもテストが通ってしまう。
    """
    repo = new_repo()
    try:
        baseline(repo)
        code, out = check(repo)
        assert code == 0, out
        assert "D08 は active だが該当文書が 0 本" in out, out
        assert "D12 は active" not in out, f"n/a クラスを充足ギャップとして警告している:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_rename_chain_is_not_stale() -> None:
    """**最重要の 2 段目**: リネームが 2 回以上重なっても stale と誤判定しない。

    R100 を飛ばすときに追跡パスをリネーム元へ巻き戻さないと、2 回目の移設で name-status の
    終点が一致せず判定が壊れる（1 段しか効かない）。`--follow` はリネームで履歴を打ち切る
    ことがあるので、log の取り直しで辿る実装であることも併せて固定する。
    """
    repo = new_repo()
    try:
        baseline(repo)
        for old, new in [("0001-first.md", "0001-r1.md"), ("0001-r1.md", "0001-r2.md")]:
            run_git(repo, "mv", f"docs/original-docs/{old}", f"docs/original-docs/{new}")
            commit_all(repo, f"パス移動のみ {old} → {new}")
        sha_before = None
        for line in (repo / "docs/knowledge/a.md").read_text(encoding="utf-8").splitlines():
            if line.startswith("distilled_from_sha:"):
                sha_before = line.split('"')[1]
        write_doc(repo, "docs/knowledge/a.md", ["D19"],
                  ["docs/original-docs/0001-r2.md"], sha_before)
        reg = repo / "docs/knowledge/doc-classes.md"
        reg.write_text(reg.read_text(encoding="utf-8").replace("0001-first.md", "0001-r2.md"),
                       encoding="utf-8")
        commit_all(repo, "sources のパスを追従")
        code, out = check(repo)
        assert code == 0, out
        assert "STALE" not in out, f"2 段のリネームを stale と誤判定した:\n{out}"
        assert "履歴が無く" not in out, f"履歴を辿れなくなっている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_frontmatter_only_change_is_not_stale() -> None:
    """source の frontmatter メタデータだけが変わった場合は stale にしない。

    ADR 移設の sources パス追従や doc_class の付与がこれ。本文が 1 文字も変わっていないのに
    「内容変更」と見なすと、それを sources に持つ knowledge が軒並み stale になる。
    """
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_doc(repo, "docs/specifications/s.md", ["D19"],
                  ["docs/original-docs/0001-first.md"], sha)
        write_registry(repo, sha, d19=2,
                       docs=[("knowledge/a.md", ["D19"]), ("specifications/s.md", ["D19"])])
        write_doc(repo, "docs/knowledge/a.md", ["D19"],
                  ["docs/specifications/s.md", "docs/original-docs/0001-first.md"], sha)
        added = commit_all(repo, "s.md を追加して a.md の source にする")
        # s.md の作成コミット自体は a.md の distill より後になるので、まず追従させる
        # （そうしないと「source が新しい」という正しい stale を拾ってしまう）。
        write_doc(repo, "docs/knowledge/a.md", ["D19"],
                  ["docs/specifications/s.md", "docs/original-docs/0001-first.md"], added)
        commit_all(repo, "a.md を s.md の追加時点まで追従")
        # s.md の frontmatter だけを変える（doc_class 追加相当）。本文は不変。
        write_doc(repo, "docs/specifications/s.md", ["D19", "D22"],
                  ["docs/original-docs/0001-first.md"], sha)
        write_registry(repo, sha, d19=2, d22=1,
                       docs=[("knowledge/a.md", ["D19"]),
                             ("specifications/s.md", ["D19", "D22"])])
        commit_all(repo, "s.md の frontmatter だけ変更")
        code, out = check(repo)
        assert code == 0, out
        assert "STALE" not in out, f"frontmatter のみの変更を stale と誤判定した:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_invalid_utf8_body_change_with_metadata_is_stale() -> None:
    """例外 1b 側の対照: 不正バイトの本文差分がメタデータ変更に相乗りしない。

    `is_metadata_only_change` は**復号後の本文を文字列で比べる**ので、復号が
    `errors="replace"` だと異なる不正バイトが同じ U+FFFD に潰れて「本文は同一」と
    判定され、frontmatter だけの変更として免除される。往復可能な surrogateescape なら
    相違が保存される。
    """
    repo = new_repo()
    try:
        baseline(repo)
        src = repo / "docs/original-docs/0001-first.md"
        body = src.read_text(encoding="utf-8")
        src.write_bytes(
            ("---\nstatus: Confirmed\ntags: [D19]\n---\n\n" + body).encode("utf-8")
            + b"\n\xff\xfe end\n"
        )
        sha = commit_all(repo, "source に frontmatter と不正バイトを置く")
        write_registry(repo, sha)
        write_doc(repo, "docs/knowledge/a.md", ["D19"], ["docs/original-docs/0001-first.md"], sha)
        commit_all(repo, "pin sha")
        assert check(repo)[0] == 0, "前提: ここでは stale でない"

        # frontmatter のメタデータ（tags）だけを変えつつ、本文の不正バイトを差し替える。
        raw = src.read_bytes()
        raw = raw.replace(b"tags: [D19]", b"tags: [D19, D22]")
        raw = raw.replace(b"\xff\xfe end", b"\xfe\xff end")
        src.write_bytes(raw)
        commit_all(repo, "メタデータ変更と不正バイトの差し替え")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"不正バイトの本文差分が U+FFFD に潰れて免除されている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_status_change_in_source_is_stale() -> None:
    """`status` / `kind` は METADATA_KEYS から**意図的に外している**ことを固定する。

    `Confirmed → Conflict` は「この知はもう信じるな」という下流へ伝えるべき信号なので、
    frontmatter だけの変更でも stale にする（例外 1b の対象外）。`METADATA_KEYS` に
    `status` が紛れ込んでも、この対照が無いと全テストが緑のまま通る。
    """
    repo = new_repo()
    try:
        baseline(repo)
        src = repo / "docs/original-docs/0001-first.md"
        body = src.read_text(encoding="utf-8")
        # source 側に frontmatter がある状態を作り、そこまでを蒸留済みとして pin する
        # （frontmatter の新規追加そのものは「本文以外の差分」ではなく追加なので内容変更になる）。
        src.write_text("---\nstatus: Confirmed\nkind: original\n---\n\n" + body, encoding="utf-8")
        sha = commit_all(repo, "source に frontmatter を付ける")
        write_registry(repo, sha)
        write_doc(repo, "docs/knowledge/a.md", ["D19"], ["docs/original-docs/0001-first.md"], sha)
        commit_all(repo, "pin sha")
        assert check(repo)[0] == 0, "前提: ここでは stale でない"

        # ここから status だけを変える（本文・他キーは不変）
        src.write_text(
            "---\nstatus: Conflict\nkind: original\n---\n\n" + body, encoding="utf-8"
        )
        commit_all(repo, "source の status だけを Conflict にする")
        code, out = check(repo)
        assert code == 1, f"status の変更は stale にするべき: {out}"
        assert "STALE" in out, out
    finally:
        shutil.rmtree(repo)


def test_body_change_is_stale() -> None:
    """対照: 本文が変われば（frontmatter が同じでも）stale になる。"""
    repo = new_repo()
    try:
        baseline(repo)
        p = repo / "docs/original-docs/0001-first.md"
        p.write_text(p.read_text(encoding="utf-8") + "\n本文の追記。\n", encoding="utf-8")
        commit_all(repo, "本文を変更")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"本文の変更を検出できていない:\n{out}"
    finally:
        shutil.rmtree(repo)


# --- 例外 1d: `uses:` のピン留め SHA 更新だけの差分 --------------------------
# ワークフローを sources に持つ文書（実物は ci-pipeline.md ← .github/workflows/ci.yml）で、
# dependabot の Actions 更新 PR が構造的に stale にならないことを固定する。

WORKFLOW_REL = ".github/workflows/ci.yml"
PIN_CHECKOUT = "a" * 40
PIN_TOOLCHAIN_OLD = "b" * 40
PIN_TOOLCHAIN_NEW = "c" * 40
PIN_CACHE_OLD = "d" * 40
PIN_CACHE_NEW = "e" * 40
PIN_REUSABLE_OLD = "f" * 40
PIN_REUSABLE_NEW = "9" * 40
PIN_COMMENT = "サプライチェーン対策で commit SHA にピン留めする。"

WORKFLOW_TEMPLATE = """\
name: CI
on: [push]
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      # {comment}
      - uses: actions/checkout@{checkout}
      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@{toolchain}
      - name: Cache cargo build
        uses: Swatinem/rust-cache@{cache} # {cache_tag}
  call:
    uses: acme/shared/.github/workflows/reusable.yml@{reusable}
"""


def write_workflow(repo: Path, toolchain: str = PIN_TOOLCHAIN_OLD, cache: str = PIN_CACHE_OLD,
                   cache_tag: str = "v2", comment: str = PIN_COMMENT,
                   checkout: str = PIN_CHECKOUT, reusable: str = PIN_REUSABLE_OLD) -> None:
    path = repo / WORKFLOW_REL
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        WORKFLOW_TEMPLATE.format(comment=comment, checkout=checkout, toolchain=toolchain,
                                 cache=cache, cache_tag=cache_tag, reusable=reusable),
        encoding="utf-8",
    )


def workflow_baseline(repo: Path) -> None:
    """a.md が sources にワークフローを持ち、error 0 で通る状態を作る。"""
    sha = baseline(repo)
    sources = [WORKFLOW_REL, "docs/original-docs/0001-first.md"]
    write_workflow(repo)
    write_doc(repo, "docs/knowledge/a.md", ["D19"], sources, sha)
    added = commit_all(repo, "ワークフローを a.md の source にする")
    # ワークフロー追加コミット自体が a.md の distill より後になるので、まず追従させる。
    write_doc(repo, "docs/knowledge/a.md", ["D19"], sources, added)
    commit_all(repo, "a.md をワークフロー追加時点まで追従")
    assert check(repo)[0] == 0, "前提: ここでは error 0 で通る"


def test_pin_hex_only_change_is_not_stale() -> None:
    """hex だけの更新（タグを切らない action。実物は dtolnay/rust-toolchain）。"""
    repo = new_repo()
    try:
        workflow_baseline(repo)
        write_workflow(repo, toolchain=PIN_TOOLCHAIN_NEW)
        commit_all(repo, "rust-toolchain のピンを更新")
        code, out = check(repo)
        assert code == 0, out
        assert "STALE" not in out, f"ピン hex のみの更新を stale と誤判定した:\n{out}"
        # 「履歴を辿れず」warning に退化しても STALE は出ないので、緑の意味を取り違えない
        # ように fail-open 経路も塞ぐ（このリポジトリが最も嫌う silent-green）。
        assert "履歴が無く" not in out, f"stale 判定が丸ごとスキップされている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_pin_bump_with_version_comment_is_not_stale() -> None:
    """hex ＋末尾のバージョン注記（`# v2` → `# v2.9.2`）。dependabot が実際に出す形。"""
    repo = new_repo()
    try:
        workflow_baseline(repo)
        write_workflow(repo, cache=PIN_CACHE_NEW, cache_tag="v2.9.2")
        commit_all(repo, "rust-cache のピンと注記を更新")
        code, out = check(repo)
        assert code == 0, out
        assert "STALE" not in out, f"末尾注記込みのピン更新を stale と誤判定した:\n{out}"
        assert "履歴が無く" not in out, f"stale 判定が丸ごとスキップされている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_multiple_pins_updated_at_once_is_not_stale() -> None:
    """1 コミットで複数のピンを同時更新（#590 と #591 を 1 本にまとめた実際の形）。"""
    repo = new_repo()
    try:
        workflow_baseline(repo)
        write_workflow(repo, toolchain=PIN_TOOLCHAIN_NEW, cache=PIN_CACHE_NEW, cache_tag="v2.9.2")
        commit_all(repo, "rust-toolchain と rust-cache を同時に更新")
        code, out = check(repo)
        assert code == 0, out
        assert "STALE" not in out, f"複数ピンの同時更新を stale と誤判定した:\n{out}"
        assert "履歴が無く" not in out, f"stale 判定が丸ごとスキップされている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_pin_comment_only_change_is_stale() -> None:
    """対照: hex が 1 文字も変わらず末尾注記だけ書き換えた変更は免除しない。

    例外の条件は「ピン留め SHA 更新のみ」なので、SHA が動いていない差分は対象外。
    """
    repo = new_repo()
    try:
        workflow_baseline(repo)
        write_workflow(repo, cache_tag="v2.9.2")  # hex は据え置き、注記だけ変える
        commit_all(repo, "注記だけを書き換え")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"hex 不変の注記変更を免除してしまっている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_pin_note_without_space_is_stale() -> None:
    """対照: `@<40hex>#v4`（`#` の前に空白なし）は YAML のコメントではないので免除しない。

    正規表現を `\\s+#` から `\\s*#` に緩めると、ref の一部を注記と誤認して免除される。
    """
    repo = new_repo()
    try:
        sha = baseline(repo)
        rel = WORKFLOW_REL
        sources = [rel, "docs/original-docs/0001-first.md"]
        (repo / rel).parent.mkdir(parents=True, exist_ok=True)
        (repo / rel).write_text(
            f"name: CI\non: [push]\njobs:\n  a:\n    steps:\n"
            f"      - uses: actions/checkout@{PIN_CHECKOUT}#v4\n",
            encoding="utf-8",
        )
        write_doc(repo, "docs/knowledge/a.md", ["D19"], sources, sha)
        added = commit_all(repo, "空白なし注記のワークフローを source にする")
        write_doc(repo, "docs/knowledge/a.md", ["D19"], sources, added)
        commit_all(repo, "追従")
        assert check(repo)[0] == 0, "前提: ここでは stale でない"

        (repo / rel).write_text(
            (repo / rel).read_text(encoding="utf-8").replace(PIN_CHECKOUT, PIN_TOOLCHAIN_NEW),
            encoding="utf-8",
        )
        commit_all(repo, "空白なし注記のまま hex を更新")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"`#` 前の空白なしを版注記として免除している:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_invalid_utf8_change_with_pin_bump_is_stale() -> None:
    """対照: 非 pin 行の不正バイトの差分がピン更新に相乗りしない。

    ここが固定するのは `blob_at` の**バイト取得**（行比較がバイトなので差分が見える）。
    復号の往復可能性を固定するのは、`uses:` 行の owner/repo に不正バイトを置く
    `test_invalid_utf8_in_owner_repo_is_stale` と、例外 1b 側の
    `test_invalid_utf8_body_change_with_metadata_is_stale`。
    """
    repo = new_repo()
    try:
        workflow_baseline(repo)
        path = repo / WORKFLOW_REL
        # 非 pin 行に不正バイトを埋めた状態を「前」とする。
        base = path.read_text(encoding="utf-8").encode("utf-8")
        path.write_bytes(base.replace(PIN_COMMENT.encode("utf-8"), b"\xff\xfe note"))
        pinned = commit_all(repo, "非 pin 行に不正バイトを置く")
        write_doc(repo, "docs/knowledge/a.md", ["D19"],
                  [WORKFLOW_REL, "docs/original-docs/0001-first.md"], pinned)
        commit_all(repo, "追従")
        assert check(repo)[0] == 0, "前提: ここでは stale でない"

        # ピン更新と同時に、不正バイトだけを別の不正バイトへ差し替える。
        raw = path.read_bytes().replace(b"\xff\xfe note", b"\xfe\xff note")
        path.write_bytes(raw.replace(PIN_TOOLCHAIN_OLD.encode(), PIN_TOOLCHAIN_NEW.encode()))
        commit_all(repo, "ピン更新と不正バイトの差し替え")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"不正バイトの差分が U+FFFD に潰れて免除されている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_invalid_utf8_in_owner_repo_is_stale() -> None:
    """対照: owner/repo に不正バイトがある行の差し替えを免除しない。

    ここが `decode_preserving` の往復可能性を固定する。`errors="replace"` だと
    `a\\xff/b` と `a\\xfe/b` がどちらも `a\\ufffd/b` に潰れ、**group(2) が一致して
    「owner/repo は同じ・hex だけ変わった」に見えるので免除される**。
    """
    repo = new_repo()
    try:
        sha = baseline(repo)
        rel = WORKFLOW_REL
        sources = [rel, "docs/original-docs/0001-first.md"]
        (repo / rel).parent.mkdir(parents=True, exist_ok=True)
        head = b"name: CI\non: [push]\njobs:\n  a:\n    steps:\n      - uses: a"
        (repo / rel).write_bytes(head + b"\xff/b@" + PIN_CHECKOUT.encode() + b"\n")
        write_doc(repo, "docs/knowledge/a.md", ["D19"], sources, sha)
        added = commit_all(repo, "owner に不正バイトを含むワークフローを source にする")
        write_doc(repo, "docs/knowledge/a.md", ["D19"], sources, added)
        commit_all(repo, "追従")
        assert check(repo)[0] == 0, "前提: ここでは stale でない"

        # owner の不正バイトを別の不正バイトへ変えつつ hex も更新する。
        (repo / rel).write_bytes(head + b"\xfe/b@" + PIN_TOOLCHAIN_NEW.encode() + b"\n")
        commit_all(repo, "owner の不正バイトと hex を同時に変更")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"owner/repo の不正バイト差分が潰れて免除されている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_pin_note_with_prose_suffix_is_stale() -> None:
    """対照: 版注記のあとに散文が続く形（`# v4だがこの版は使わない`）は免除しない。

    `\\w` は Unicode を含むので、文字クラスを ASCII に絞らないと「版注記の形」として通る。
    """
    repo = new_repo()
    try:
        workflow_baseline(repo)
        write_workflow(repo, cache=PIN_CACHE_NEW, cache_tag="v2.9.2だがこの版は使わない")
        commit_all(repo, "ピン更新と注記への散文追記")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"版注記のあとの散文を免除してしまっている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_pin_shortened_to_short_sha_is_stale() -> None:
    """対照: 40 hex から短縮 SHA への緩和はサプライチェーン対策の後退なので内容変更。

    量指定子を `{7,40}` のように緩めると通ってしまうので固定する。
    """
    repo = new_repo()
    try:
        workflow_baseline(repo)
        path = repo / WORKFLOW_REL
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                f"dtolnay/rust-toolchain@{PIN_TOOLCHAIN_OLD}", "dtolnay/rust-toolchain@abc1234"),
            encoding="utf-8",
        )
        commit_all(repo, "ピンを短縮 SHA へ緩める")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"短縮 SHA への緩和を免除してしまっている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_pin_note_replaced_by_prose_is_stale() -> None:
    """対照: 末尾注記を版の形でない散文へ差し替えたら内容変更。

    group(4) を任意コメントにすると、ピン更新に紛れて注記へ何を書いても免除される。
    """
    repo = new_repo()
    try:
        workflow_baseline(repo)
        write_workflow(repo, cache=PIN_CACHE_NEW, cache_tag="この版は上げない（暫定）")
        commit_all(repo, "ピン更新と注記の散文化")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"版注記でない末尾コメントを免除してしまっている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_pin_indent_change_is_stale() -> None:
    """対照: ピン行のインデントが変わったら内容変更（group(1) の比較を消すと通る）。"""
    repo = new_repo()
    try:
        workflow_baseline(repo)
        path = repo / WORKFLOW_REL
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                f"        uses: dtolnay/rust-toolchain@{PIN_TOOLCHAIN_OLD}",
                f"          uses: dtolnay/rust-toolchain@{PIN_TOOLCHAIN_NEW}"),
            encoding="utf-8",
        )
        commit_all(repo, "ピン更新とインデント変更")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"インデント変更を免除してしまっている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_reusable_workflow_pin_change_is_stale() -> None:
    """対照: 再利用可能ワークフロー参照の SHA 更新は呼び先のジョブ構成ごと変わる。

    owner/repo を 2 要素に絞らないと `owner/repo/.github/workflows/x.yml@<sha>` まで拾う。
    """
    repo = new_repo()
    try:
        workflow_baseline(repo)
        write_workflow(repo, reusable=PIN_REUSABLE_NEW)
        commit_all(repo, "再利用可能ワークフローのピンを更新")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"再利用可能ワークフローの更新を免除してしまっている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_crlf_only_change_in_workflow_is_stale() -> None:
    """対照: 改行コードだけの変更も内容変更として扱う（保守側）。

    機序は「差分行が 0 件」ではない——LF → CRLF では**全行が `\\r` 付きで差分になり**、
    その差分行が `RE_USES_PIN` に合わないので免除されない（hex も動いていない）。
    なお**このケースはバイト列比較の mutation 検出力を持たない**（`text=True` に戻しても
    hex が動いていないので非免除のまま）。バイト取得を固定するのは
    `test_crlf_conversion_with_pin_bump_is_stale`。
    """
    repo = new_repo()
    try:
        workflow_baseline(repo)
        path = repo / WORKFLOW_REL
        path.write_bytes(path.read_text(encoding="utf-8").replace("\n", "\r\n").encode("utf-8"))
        commit_all(repo, "改行コードを CRLF にする")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"改行コードのみの変更を免除してしまっている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_pin_form_in_markdown_source_is_stale() -> None:
    """対照: 例外 1d の対象はワークフローだけ。Markdown 中のピン形の行は免除しない。

    絞らないと、コードフェンスに書いた `uses:` の見本を書き換えただけでその文書の
    stale 検査が消える（判定は行単位・字面ベースで YAML 構造を見ないため）。
    """
    repo = new_repo()
    try:
        baseline(repo)
        src = repo / "docs/original-docs/0001-first.md"
        fence = f"\n```yaml\n      - uses: actions/checkout@{PIN_CHECKOUT}\n```\n"
        src.write_text(src.read_text(encoding="utf-8") + fence, encoding="utf-8")
        sha = commit_all(repo, "source にワークフローの見本を足す")
        write_registry(repo, sha)
        write_doc(repo, "docs/knowledge/a.md", ["D19"], ["docs/original-docs/0001-first.md"], sha)
        commit_all(repo, "pin sha")
        assert check(repo)[0] == 0, "前提: ここでは stale でない"

        src.write_text(
            src.read_text(encoding="utf-8").replace(PIN_CHECKOUT, PIN_TOOLCHAIN_NEW),
            encoding="utf-8",
        )
        commit_all(repo, "見本の hex だけを書き換え")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"Markdown 中のピン形の行を免除してしまっている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_pin_commits_beyond_window_do_not_hide_stale() -> None:
    """走査窓（limit=40）をピン更新で埋めても、その先の実内容変更を見失わない。

    None は呼び出し側で warning になり stale 判定がスキップされる（fail-open）。免除対象が
    機械生成のコミットになった以上、「窓を埋めれば検査が消える」経路を残せない。
    """
    repo = new_repo()
    try:
        workflow_baseline(repo)
        # 追従させない実内容変更。これが「最後の内容変更」であり続けるべき。
        path = repo / WORKFLOW_REL
        path.write_text(
            path.read_text(encoding="utf-8").replace(PIN_COMMENT, "説明を書き換える"),
            encoding="utf-8",
        )
        commit_all(repo, "ワークフローの説明を実質変更")
        assert "STALE" in check(repo)[1], "前提: ここで stale になる"

        # 窓（40 件）を超える数のピン更新を積む。
        for i in range(41):
            write_workflow(repo, toolchain=f"{i:040x}", comment="説明を書き換える")
            commit_all(repo, f"ピン更新 {i}")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"窓の枯渇で stale 判定が消えている（fail-open）:\n{out}"
        assert "履歴が無く" not in out, f"窓の枯渇を履歴の尽きと混同している:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_pin_change_with_comment_edit_is_stale() -> None:
    """対照: ピン更新に説明コメントの改訂が同居したら内容変更（#607 の a5cfa46 がこの形）。

    例外が広すぎないことの担保。ここが緑のまま通ると「ci.yml を触った PR は何でも
    stale にならない」に退行する。
    """
    repo = new_repo()
    try:
        workflow_baseline(repo)
        write_workflow(repo, toolchain=PIN_TOOLCHAIN_NEW,
                       comment="ピンの更新は dependabot の PR で行い、固定した日付はここに書かない。")
        commit_all(repo, "ピン更新と説明コメントの改訂")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"コメント改訂込みの変更を検出できていない:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_action_repo_change_is_stale() -> None:
    """対照: owner/repo の差し替えは別の action を呼ぶこと＝ジョブの意味が変わる。

    **hex も同時に変える。** 据え置くと hex_changed が立たない側で弾かれてしまい、
    owner/repo の同一性判定（group(1, 2) の比較）を固定できない（実際の差し替えも
    ピン更新と同居する形になる）。
    """
    repo = new_repo()
    try:
        workflow_baseline(repo)
        path = repo / WORKFLOW_REL
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                f"dtolnay/rust-toolchain@{PIN_TOOLCHAIN_OLD}",
                f"actions-rust-lang/setup-rust-toolchain@{PIN_TOOLCHAIN_NEW}"),
            encoding="utf-8",
        )
        commit_all(repo, "toolchain の action を差し替え（ピン更新と同居）")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"action の差し替えを検出できていない:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_workflow_line_added_with_pin_bump_is_stale() -> None:
    """挙動の固定: ステップ追加とピン更新が同居しても内容変更。

    **これは行数一致ガードの mutation 検出ではない。** 末尾に内容のある行を足すと `zip` が
    短い側で切れて末尾がズレ、そこに非ピン差分が生まれるのでガードが無くても非免除になる。
    ガードそのものを固定するのは `test_trailing_blank_line_with_pin_bump_is_stale`。
    """
    repo = new_repo()
    try:
        workflow_baseline(repo)
        write_workflow(repo, toolchain=PIN_TOOLCHAIN_NEW)
        path = repo / WORKFLOW_REL
        path.write_text(
            path.read_text(encoding="utf-8") + "      - run: cargo test\n", encoding="utf-8"
        )
        commit_all(repo, "ピン更新とステップ追加")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"ピン更新＋ステップ追加を免除してしまっている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_trailing_blank_line_with_pin_bump_is_stale() -> None:
    """対照: 行数一致ガードを固定する唯一の形（末尾に空行 1 行＋ピン更新）。

    末尾に**内容のある**行を足す形ではガードを固定できない——`zip` が短い側で切れて
    末尾がズレ、そこに非ピン差分が生まれるのでガードが無くても内容変更に落ちる。
    空行なら zip の範囲内が完全一致するため、**ガードだけが唯一の防壁**になる。
    """
    repo = new_repo()
    try:
        workflow_baseline(repo)
        write_workflow(repo, toolchain=PIN_TOOLCHAIN_NEW)
        path = repo / WORKFLOW_REL
        path.write_text(path.read_text(encoding="utf-8") + "\n", encoding="utf-8")
        commit_all(repo, "ピン更新と末尾の空行追加")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"行数の増加を免除してしまっている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_crlf_conversion_with_pin_bump_is_stale() -> None:
    """対照: CRLF 変換とピン更新が同居しても免除しない。

    blob 取得が universal newlines を通ると `\\r` が消えて「ピン行だけの差分」に見える。
    `run:` ブロックの改行コードは shell の挙動を変えうるので、内容変更として扱う。
    """
    repo = new_repo()
    try:
        workflow_baseline(repo)
        write_workflow(repo, toolchain=PIN_TOOLCHAIN_NEW)
        path = repo / WORKFLOW_REL
        path.write_bytes(path.read_text(encoding="utf-8").replace("\n", "\r\n").encode("utf-8"))
        commit_all(repo, "ピン更新と CRLF 変換")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"CRLF 変換込みのピン更新を免除してしまっている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_pin_bump_with_note_only_line_is_not_stale() -> None:
    """免除する形の明文化: 1 行は hex 更新、別の 1 行は注記だけの変更。

    hex_changed は「どこか 1 行で hex が動いた」なので、同じコミット内の別のピン行が
    注記だけの変更でも免除される。意図した挙動なので固定しておく。
    """
    repo = new_repo()
    try:
        workflow_baseline(repo)
        write_workflow(repo, toolchain=PIN_TOOLCHAIN_NEW, cache_tag="v2.9.2")
        commit_all(repo, "片方は hex 更新・片方は注記だけ")
        code, out = check(repo)
        assert code == 0, out
        assert "STALE" not in out, f"混在した免除対象を stale と誤判定した:\n{out}"
        assert "履歴が無く" not in out, f"stale 判定が丸ごとスキップされている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_pin_change_in_yaml_extension_workflow_is_not_stale() -> None:
    """`.yaml` 拡張子のワークフローも例外 1d の対象（RE_WORKFLOW_PATH の `ya?ml`）。"""
    repo = new_repo()
    try:
        sha = baseline(repo)
        rel = ".github/workflows/audit.yaml"
        sources = [rel, "docs/original-docs/0001-first.md"]
        (repo / rel).parent.mkdir(parents=True, exist_ok=True)
        (repo / rel).write_text(
            f"name: audit\non: [push]\njobs:\n  a:\n    steps:\n"
            f"      - uses: actions/checkout@{PIN_CHECKOUT}\n",
            encoding="utf-8",
        )
        write_doc(repo, "docs/knowledge/a.md", ["D19"], sources, sha)
        added = commit_all(repo, "audit.yaml を source にする")
        write_doc(repo, "docs/knowledge/a.md", ["D19"], sources, added)
        commit_all(repo, "追従")
        assert check(repo)[0] == 0, "前提: ここでは stale でない"

        (repo / rel).write_text(
            (repo / rel).read_text(encoding="utf-8").replace(PIN_CHECKOUT, PIN_TOOLCHAIN_NEW),
            encoding="utf-8",
        )
        commit_all(repo, "audit.yaml のピンを更新")
        code, out = check(repo)
        assert code == 0, out
        assert "STALE" not in out, f".yaml 拡張子が例外 1d の対象から外れている:\n{out}"
        assert "履歴が無く" not in out, f"stale 判定が丸ごとスキップされている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_pin_change_in_non_workflow_yml_is_stale() -> None:
    """対照: ワークフロー以外の `.yml`（compose 等）は例外 1d の対象外。"""
    repo = new_repo()
    try:
        sha = baseline(repo)
        rel = "deployments/compose.yml"
        sources = [rel, "docs/original-docs/0001-first.md"]
        (repo / rel).parent.mkdir(parents=True, exist_ok=True)
        (repo / rel).write_text(
            f"services:\n  a:\n    steps:\n      - uses: actions/checkout@{PIN_CHECKOUT}\n",
            encoding="utf-8",
        )
        write_doc(repo, "docs/knowledge/a.md", ["D19"], sources, sha)
        added = commit_all(repo, "compose.yml を source にする")
        write_doc(repo, "docs/knowledge/a.md", ["D19"], sources, added)
        commit_all(repo, "追従")
        assert check(repo)[0] == 0, "前提: ここでは stale でない"

        (repo / rel).write_text(
            (repo / rel).read_text(encoding="utf-8").replace(PIN_CHECKOUT, PIN_TOOLCHAIN_NEW),
            encoding="utf-8",
        )
        commit_all(repo, "compose.yml のピン形の行を更新")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"ワークフロー以外の .yml を免除してしまっている:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_pin_to_tag_change_is_stale() -> None:
    """対照: 40 hex → タグへの緩和はサプライチェーン対策の後退なので内容変更。"""
    repo = new_repo()
    try:
        workflow_baseline(repo)
        path = repo / WORKFLOW_REL
        path.write_text(
            path.read_text(encoding="utf-8").replace(f"actions/checkout@{PIN_CHECKOUT}",
                                                     "actions/checkout@v4"),
            encoding="utf-8",
        )
        commit_all(repo, "checkout のピンをタグへ緩める")
        code, out = check(repo)
        assert code == 1, out
        assert "STALE" in out, f"ピンからタグへの緩和を検出できていない:\n{out}"
    finally:
        shutil.rmtree(repo)


# --- 走査窓のページングとリネームの相互作用 -----------------------------------
# ここだけは checker をモジュールとして読み込んで `last_content_change` を直接叩く。
# limit / max_pages を小さくできるので、窓の枯渇や打ち切りを 40 コミット積まずに固定できる。
# **テストごとに読み込み直す**のが要点で、`last_content_change` の lru_cache はキーに
# リポジトリを含まないため、モジュールを共有すると別の一時リポの結果を引いてしまう。


def load_checker(repo: Path) -> "types.ModuleType":
    spec = importlib.util.spec_from_file_location("checker_under_test", TARGET)
    assert spec is not None and spec.loader is not None, TARGET
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module._ROOT = repo
    return module


def write_moved_workflow(repo: Path, rel: str, toolchain: str, comment: str) -> None:
    (repo / rel).write_text(
        WORKFLOW_TEMPLATE.format(comment=comment, checkout=PIN_CHECKOUT, toolchain=toolchain,
                                 cache=PIN_CACHE_OLD, cache_tag="v2", reusable=PIN_REUSABLE_OLD),
        encoding="utf-8",
    )


def test_paging_finds_content_change_beyond_window() -> None:
    """除外対象が窓を埋めても、その先の内容変更を見つける。"""
    repo = new_repo()
    try:
        m = load_checker(repo)
        write_workflow(repo)
        commit_all(repo, "ワークフローを追加")
        write_workflow(repo, comment="説明を書き換える")
        commit_all(repo, "説明を実質変更")
        real = run_git(repo, "rev-parse", "HEAD")
        for i in range(5):
            write_workflow(repo, toolchain=f"{i:040x}", comment="説明を書き換える")
            commit_all(repo, f"ピン更新 {i}")
        got = m.last_content_change(WORKFLOW_REL, limit=2, max_pages=10)
        assert got == real, f"窓の先にある内容変更を見失っている: {got} != {real}"
    finally:
        shutil.rmtree(repo)


def test_metadata_only_commits_beyond_window_do_not_hide_stale() -> None:
    """ページングは例外 1b（frontmatter のみの変更）にも効く。"""
    repo = new_repo()
    try:
        m = load_checker(repo)
        rel = "docs/specifications/s.md"
        write_doc(repo, rel, ["D19"], ["docs/original-docs/0001-first.md"], "HEAD")
        commit_all(repo, "s.md を追加")
        p = repo / rel
        p.write_text(p.read_text(encoding="utf-8") + "\n本文の追記。\n", encoding="utf-8")
        commit_all(repo, "s.md の本文を変更")
        real = run_git(repo, "rev-parse", "HEAD")
        for i in range(5):
            p.write_text(
                re.sub(r'distilled_from_sha: "[^"]*"', f'distilled_from_sha: "{i:07x}"',
                       p.read_text(encoding="utf-8")),
                encoding="utf-8",
            )
            commit_all(repo, f"sha だけ更新 {i}")
        got = m.last_content_change(rel, limit=2, max_pages=10)
        assert got == real, f"frontmatter のみのコミットで窓が埋まると見失う: {got} != {real}"
    finally:
        shutil.rmtree(repo)


def test_page_budget_exhaustion_is_reported_as_aborted() -> None:
    """ページ予算を使い切ったら ScanAborted を返す（None＝履歴が無い とは別物）。

    ここを None に混ぜると、呼び出し側が warning に落として stale 判定をスキップし、
    「除外対象のコミットを積めば検査が消える」fail-open が一段外側で再現する。
    """
    repo = new_repo()
    try:
        m = load_checker(repo)
        write_workflow(repo)
        commit_all(repo, "ワークフローを追加")
        write_workflow(repo, comment="説明を書き換える")
        commit_all(repo, "説明を実質変更")
        for i in range(6):
            write_workflow(repo, toolchain=f"{i:040x}", comment="説明を書き換える")
            commit_all(repo, f"ピン更新 {i}")
        got = m.last_content_change(WORKFLOW_REL, limit=2, max_pages=2)
        assert isinstance(got, m.ScanAborted), f"打ち切りを履歴の尽きと混同している: {got!r}"
        assert "max_pages" in got.reason, f"原因がページ予算だと分からない: {got.reason}"
    finally:
        shutil.rmtree(repo)


def test_rename_budget_exhaustion_is_reported_as_aborted() -> None:
    """リネーム予算の上限で打ち切ったときも ScanAborted（None に混ぜない）。

    ページ予算の枝とは別の経路なので、片方だけ直しても他方が warning に落ちる。
    原因の取り違えを防ぐため、reason にどちらの予算かを載せることも固定する。
    """
    repo = new_repo()
    try:
        m = load_checker(repo)
        write_workflow(repo)
        commit_all(repo, "ワークフローを追加")
        first = ".github/workflows/ci-1.yml"
        second = ".github/workflows/ci-2.yml"
        run_git(repo, "mv", WORKFLOW_REL, first)
        commit_all(repo, "1 回目の改名（内容不変）")
        run_git(repo, "mv", first, second)
        commit_all(repo, "2 回目の改名（内容不変）")
        got = m.last_content_change(second, limit=5, max_renames=1)
        assert isinstance(got, m.ScanAborted), f"リネーム上限の打ち切りを None に混ぜている: {got!r}"
        assert "max_renames" in got.reason, f"原因がリネーム予算だと分からない: {got.reason}"
    finally:
        shutil.rmtree(repo)


def test_rename_within_budget_still_finds_change() -> None:
    """境界の対照: 予算内のリネームは打ち切らず、改名前の内容変更まで辿る。"""
    repo = new_repo()
    try:
        m = load_checker(repo)
        write_workflow(repo)
        commit_all(repo, "ワークフローを追加")
        write_workflow(repo, comment="説明を書き換える")
        commit_all(repo, "説明を実質変更")
        real = run_git(repo, "rev-parse", "HEAD")
        moved = ".github/workflows/ci-1.yml"
        run_git(repo, "mv", WORKFLOW_REL, moved)
        commit_all(repo, "1 回だけ改名（内容不変）")
        got = m.last_content_change(moved, limit=5, max_renames=2)
        assert got == real, f"予算内のリネームで打ち切っている: {got!r}"
    finally:
        shutil.rmtree(repo)


def test_git_log_failure_is_reported_as_aborted() -> None:
    """`git log` 自体の失敗は警告に落とさない（検査が回っていないので error 側）。"""
    repo = new_repo()
    try:
        m = load_checker(repo)
        write_workflow(repo)
        commit_all(repo, "ワークフローを追加")
        real_git = m.git

        def failing_git(*args: str):
            if args and args[0] == "log":
                return subprocess.CompletedProcess(list(args), 128, "", "fatal: 壊れた")
            return real_git(*args)

        m.git = failing_git
        got = m.last_content_change(WORKFLOW_REL)
        assert isinstance(got, m.ScanAborted), f"git log の失敗を None に落としている: {got!r}"
        assert "git log" in got.reason, f"原因が git log の失敗だと分からない: {got.reason}"
    finally:
        shutil.rmtree(repo)


def test_page_budget_is_shared_across_renames() -> None:
    """ページ予算は**走査全体**で数える（リネームで取り直さない）。

    パス単位にすると実際の上限が `max_renames × max_pages` に膨らみ、宣言した値と乖離する。
    ここでは改名後のパスで予算を使い切らせ、改名前へ移った直後に打ち切られること
    （＝`None` に落ちて warning で流れるのではなく error になること）を固定する。
    """
    repo = new_repo()
    try:
        m = load_checker(repo)
        write_workflow(repo)
        commit_all(repo, "ワークフローを追加")
        write_workflow(repo, comment="説明を書き換える")
        commit_all(repo, "説明を実質変更")
        moved = ".github/workflows/ci-renamed.yml"
        run_git(repo, "mv", WORKFLOW_REL, moved)
        commit_all(repo, "改名（内容不変）")
        # 改名後のパスでページ予算（limit=2 × max_pages=2）を使い切らせる。
        for i in range(3):
            write_moved_workflow(repo, moved, f"{i + 10:040x}", "説明を書き換える")
            commit_all(repo, f"改名後のピン更新 {i}")
        got = m.last_content_change(moved, limit=2, max_pages=2)
        assert isinstance(got, m.ScanAborted), f"予算を使い切ったのに打ち切っていない: {got!r}"
        assert "max_pages" in got.reason, got.reason
    finally:
        shutil.rmtree(repo)


def test_git_show_failure_is_reported_as_aborted() -> None:
    """`git show --name-status` の失敗も warning に落とさない。

    失敗を「このコミットは対象パスを触っていない」と同じ扱いにすると、最後の内容変更
    コミットが黙って飛んで **より古い SHA が返り stale 検査が静かに通る**。`git log` の
    失敗だけを error にしても、走査中に最も多く呼ぶこちらが飲み込めば意味がない。
    """
    repo = new_repo()
    try:
        m = load_checker(repo)
        write_workflow(repo)
        commit_all(repo, "ワークフローを追加")
        real_git = m.git

        def failing_git(*args: str):
            if "--name-status" in args:
                return subprocess.CompletedProcess(list(args), 128, "", "fatal: 壊れた")
            return real_git(*args)

        m.git = failing_git
        got = m.last_content_change(WORKFLOW_REL)
        assert isinstance(got, m.ScanAborted), f"git show の失敗を飲み込んでいる: {got!r}"
        assert "name-status" in got.reason, f"原因が分からない: {got.reason}"
    finally:
        shutil.rmtree(repo)


def test_aborted_scan_is_reported_as_error_not_warning() -> None:
    """走査の未完遂は **error**（warning に落とすと fail-open が一段外側で再現する）。

    既定の予算（limit=40 × max_pages=25）を実リポジトリで使い切らせるのは非現実的なので、
    `last_content_change` を差し替えて呼び出し側の分岐だけを固定する。
    """
    repo = new_repo()
    try:
        m = load_checker(repo)
        baseline(repo)
        m.last_content_change = lambda *a, **k: m.ScanAborted("テスト用の打ち切り")
        cwd = Path.cwd()
        out = io.StringIO()
        try:
            os.chdir(repo)
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
                code = m.main(["check"])
        finally:
            os.chdir(cwd)
        text = out.getvalue()
        assert code == 1, f"走査の未完遂が error になっていない（exit {code}）:\n{text}"
        assert "テスト用の打ち切り" in text, f"打ち切りの理由が出力されていない:\n{text}"
        assert "履歴が無く" not in text, f"打ち切りを履歴の尽きと同じ文言で流している:\n{text}"
    finally:
        shutil.rmtree(repo)


def test_window_position_resets_after_rename() -> None:
    """リネームを辿るときに窓の位置（skip）を取り直す。

    取り直さないと、改名前のパスの履歴を skip 件ぶん飛ばして読み始め、実質の変更点を
    丸ごと通り過ぎる（結果は None＝fail-open）。
    """
    repo = new_repo()
    try:
        m = load_checker(repo)
        write_workflow(repo)
        commit_all(repo, "ワークフローを追加")
        write_workflow(repo, comment="説明を書き換える")
        commit_all(repo, "説明を実質変更")
        real = run_git(repo, "rev-parse", "HEAD")
        moved = ".github/workflows/ci-renamed.yml"
        run_git(repo, "mv", WORKFLOW_REL, moved)
        commit_all(repo, "ワークフローを改名（内容不変）")
        # 改名後のパスでピン更新を積み、リネームが 2 ページ目に来る形にする。
        for i in range(3):
            write_moved_workflow(repo, moved, f"{i:040x}", "説明を書き換える")
            commit_all(repo, f"改名後のピン更新 {i}")
        got = m.last_content_change(moved, limit=2, max_pages=10)
        assert got == real, f"リネーム後に窓の位置を取り直せていない: {got} != {real}"
    finally:
        shutil.rmtree(repo)


def test_runs_from_subdirectory() -> None:
    """リポジトリルート以外を cwd にしても結果が変わらない。

    git をルートで実行しないと pathspec が cwd 相対に解決され、stale 判定が全件無音で
    スキップされたまま「✓ 整合を確認」と表示して exit 0 する（fail-open）。
    """
    repo = new_repo()
    try:
        baseline(repo)
        p = repo / "docs/original-docs/0001-first.md"
        p.write_text(p.read_text(encoding="utf-8") + "\n本文の追記。\n", encoding="utf-8")
        commit_all(repo, "本文を変更")
        proc = subprocess.run(
            [sys.executable, str(TARGET)], cwd=repo / "docs", capture_output=True, text=True
        )
        out = proc.stdout + proc.stderr
        assert "STALE" in out, f"サブディレクトリから実行すると stale 判定が消える:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_unresolvable_sha_is_error() -> None:
    """full clone で distilled_from_sha を解決できないのは error（その文書の判定が消えるため）。"""
    repo = new_repo()
    try:
        baseline(repo)
        write_doc(repo, "docs/knowledge/a.md", ["D19"],
                  ["docs/original-docs/0001-first.md"], "deadbee")
        code, out = check(repo)
        assert code == 1, f"解決不能な sha を素通りさせている:\n{out}"
        assert "を解決できない" in out, out
    finally:
        shutil.rmtree(repo)


def test_absolute_source_path_is_error() -> None:
    """sources の絶対パスは error。Path(root) / "/etc/x" は root を捨てて外を指す。"""
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_doc(repo, "docs/knowledge/a.md", ["D19"], ["/etc/hosts"], sha)
        code, out = check(repo)
        assert code == 1, out
        assert "リポジトリ相対パスで書く" in out, out
    finally:
        shutil.rmtree(repo)


def test_duplicate_class_is_error() -> None:
    """doc_class の重複は「現行」列を二重計上するので error。"""
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_doc(repo, "docs/knowledge/a.md", ["D19", "D19"],
                  ["docs/original-docs/0001-first.md"], sha)
        code, out = check(repo)
        assert code == 1, out
        assert "doc_class に重複がある" in out, out
    finally:
        shutil.rmtree(repo)


def test_inline_comment_in_flow_list_is_accepted() -> None:
    """規約が示すテンプレはインラインコメント付き。それを許さないと正本が正本を通らない。"""
    repo = new_repo()
    try:
        sha = baseline(repo)
        (repo / "docs/knowledge/a.md").write_text(
            f'---\nstatus: Confirmed\nkind: knowledge\n'
            f'doc_class: [D19]   # 第 1 要素が主クラス\ntags: [D19]        # mdq 用ミラー\n'
            f'sources:\n  - docs/original-docs/0001-first.md\n'
            f'distilled_from_sha: "{sha}"\nupdated: "2026-08-09"\n---\n\n# a\n',
            encoding="utf-8",
        )
        code, out = check(repo)
        assert code == 0, f"インラインコメント付きのテンプレが通らない:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_malformed_class_row_is_error() -> None:
    """クラス一覧の書式が崩れた行を黙って落とすと、そのクラスが「未定義」になって原因が読めない。"""
    repo = new_repo()
    try:
        baseline(repo)
        reg = repo / "docs/knowledge/doc-classes.md"
        reg.write_text(
            reg.read_text(encoding="utf-8").replace(
                "| D08 | データモデル | active | 0 |", "| D08 | データモデル | **active** | 0 |"
            ),
            encoding="utf-8",
        )
        code, out = check(repo)
        assert code == 1, out
        assert "クラス一覧の書式が崩れている" in out, out
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


# --- REQ（要件 ID）の検査 ---------------------------------------------------
# 規約は docs/knowledge/README.md「REQ-ID（要件 ID）の規約」。番号の一意性と
# 「検証手段の無い Confirmed を作らせない」が本体で、どちらも壊れても本番 docs は
# 正常なまま静かに素通りする（＝ fixture で固定する価値がある）。


def append_req_block(repo: Path, rel: str, cls: str, rows: "list[str]",
                     header: bool = True, close: "str | None" = "same") -> None:
    """既存文書の末尾に REQ ブロックを足す。close=None なら閉じない。"""
    text = (repo / rel).read_text(encoding="utf-8")
    block = [f"<!-- REQ:begin {cls} -->"]
    if header:
        block += ["| REQ-ID | 要件 | 検証手段 | 出典 | status |", "|---|---|---|---|---|"]
    block += rows
    if close is not None:
        block.append(f"<!-- REQ:end {cls if close == 'same' else close} -->")
    (repo / rel).write_text(text + "\n" + "\n".join(block) + "\n", encoding="utf-8")


VALID_ROW = "| REQ-D19-001 | 何かを満たす | `cargo test` で固定 | ADR 0001 | Confirmed |"


def test_req_valid_block_passes() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19", [VALID_ROW])
        code, out = check(repo)
        assert code == 0, f"正当な REQ 表で落ちた: {out}"
    finally:
        shutil.rmtree(repo)


def test_req_duplicate_id_is_error() -> None:
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_doc(repo, "docs/knowledge/b.md", ["D19"], ["docs/original-docs/0001-first.md"], sha)
        write_registry(repo, sha, d19=2,
                       docs=[("knowledge/a.md", ["D19"]), ("knowledge/b.md", ["D19"])])
        append_req_block(repo, "docs/knowledge/a.md", "D19", [VALID_ROW])
        append_req_block(repo, "docs/knowledge/b.md", "D19", [VALID_ROW])
        code, out = check(repo)
        assert code == 1, out
        assert "重複" in out and "REQ-D19-001" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_malformed_id_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         ["| REQ-D19-1 | 何か | `cargo test` | ADR 0001 | Confirmed |"])
        code, out = check(repo)
        assert code == 1, out
        assert "REQ-ID の形式が不正" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_class_mismatch_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         ["| REQ-D22-001 | 何か | `cargo test` | ADR 0001 | Confirmed |"])
        code, out = check(repo)
        assert code == 1, out
        assert "クラスが REQ ブロック" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_block_class_outside_doc_class_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D22",
                         ["| REQ-D22-001 | 何か | `cargo test` | ADR 0001 | Confirmed |"])
        code, out = check(repo)
        assert code == 1, out
        assert "doc_class に含まれていない" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_confirmed_without_verification_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         ["| REQ-D19-001 | 何か | - | ADR 0001 | Confirmed |"])
        code, out = check(repo)
        assert code == 1, out
        assert "検証手段が空のまま Confirmed" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_tentative_without_verification_is_ok() -> None:
    """検証手段が無いこと自体は許す。禁じるのは「測り方が無いのに Confirmed」だけ。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         ["| REQ-D19-001 | 何か | - | ADR 0001 | Tentative |"])
        code, out = check(repo)
        assert code == 0, out
    finally:
        shutil.rmtree(repo)


def test_req_unknown_status_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         ["| REQ-D19-001 | 何か | `cargo test` | ADR 0001 | Approved |"])
        code, out = check(repo)
        assert code == 1, out
        assert "status が不正" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_wrong_column_count_is_error() -> None:
    """列が欠けた行を黙って落とすと、その要件が一意性検査から消えて重複が通る。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         ["| REQ-D19-001 | 何か | `cargo test` | Confirmed |"])
        code, out = check(repo)
        assert code == 1, out
        assert "列数が 5 でない" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_unclosed_block_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19", [VALID_ROW], close=None)
        code, out = check(repo)
        assert code == 1, out
        assert "閉じられていない" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_end_class_mismatch_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19", [VALID_ROW], close="D22")
        code, out = check(repo)
        assert code == 1, out
        assert "クラスが違う" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_empty_block_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19", [])
        code, out = check(repo)
        assert code == 1, out
        assert "要件行が 1 つも無い" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_broken_link_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(
            repo, "docs/knowledge/a.md", "D19",
            ["| REQ-D19-001 | 何か | `cargo test` "
             "| [ADR 0099](../original-docs/0099-nope.md) | Confirmed |"],
        )
        code, out = check(repo)
        assert code == 1, out
        assert "リンク先が実在しない" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_link_to_existing_file_passes() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(
            repo, "docs/knowledge/a.md", "D19",
            ["| REQ-D19-001 | 何か | `cargo test` "
             "| [ADR 0001](../original-docs/0001-first.md) | Confirmed |"],
        )
        code, out = check(repo)
        assert code == 0, out
    finally:
        shutil.rmtree(repo)


def test_req_marker_in_code_fence_is_ignored() -> None:
    """規約の説明文（コードフェンス内の見本）を実データとして検査しない。"""
    repo = new_repo()
    try:
        baseline(repo)
        path = repo / "docs/knowledge/a.md"
        path.write_text(
            path.read_text(encoding="utf-8")
            + "\n```markdown\n<!-- REQ:begin D22 -->\n"
            + "| REQ-D99-9 | 見本 |  |  | Bogus |\n<!-- REQ:end D22 -->\n```\n",
            encoding="utf-8",
        )
        code, out = check(repo)
        assert code == 0, f"フェンス内の見本で落ちた: {out}"
    finally:
        shutil.rmtree(repo)


def test_req_row_outside_block_is_error() -> None:
    """マーカーを付け忘れた REQ 表を素通りさせない（表ごと無検査になる経路）。"""
    repo = new_repo()
    try:
        baseline(repo)
        path = repo / "docs/knowledge/a.md"
        path.write_text(path.read_text(encoding="utf-8") + "\n" + VALID_ROW + "\n",
                        encoding="utf-8")
        code, out = check(repo)
        assert code == 1, out
        assert "マーカーの外にある" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_malformed_marker_is_error() -> None:
    """begin/end が揃って綴り違いだと、従来は表ごと無検査で exit 0 になっていた。"""
    repo = new_repo()
    try:
        baseline(repo)
        path = repo / "docs/knowledge/a.md"
        path.write_text(
            path.read_text(encoding="utf-8")
            + "\n<!-- REQ:begin D1 -->\n" + VALID_ROW + "\n<!-- REQ:end D1 -->\n",
            encoding="utf-8",
        )
        code, out = check(repo)
        assert code == 1, out
        assert "マーカーの書式が不正" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_missing_header_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19", [VALID_ROW], header=False)
        code, out = check(repo)
        assert code == 1, out
        assert "見出し行が" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_header_order_swapped_is_error() -> None:
    """列は位置で読むので、順序が入れ替わると Confirmed 検査が別の列に当たる。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(
            repo, "docs/knowledge/a.md", "D19",
            ["| REQ-ID | 要件 | 出典 | 検証手段 | status |",
             "|---|---|---|---|---|",
             VALID_ROW],
            header=False,
        )
        code, out = check(repo)
        assert code == 1, out
        assert "見出し行が" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_absolute_link_is_error() -> None:
    """絶対パスを許すと Path 連結が root を捨て、リポジトリ外が実在扱いで通る。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         ["| REQ-D19-001 | 何か | `cargo test` "
                          "| [外](/etc/hosts) | Confirmed |"])
        code, out = check(repo)
        assert code == 1, out
        assert "相対パスで書く" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_link_outside_repo_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         ["| REQ-D19-001 | 何か | `cargo test` "
                          "| [外](../../../../etc/hosts) | Confirmed |"])
        code, out = check(repo)
        assert code == 1, out
        assert "リポジトリ外" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_escaped_pipe_in_cell_is_accepted() -> None:
    """検証手段にはコマンドを書く。`\\|` を割ると正当なセルが列数エラーに化ける。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         [r"| REQ-D19-001 | 何か | `cargo test \| tail -1` "
                          r"| ADR 0001 | Confirmed |"])
        code, out = check(repo)
        assert code == 0, f"エスケープされたパイプで落ちた: {out}"
    finally:
        shutil.rmtree(repo)


def test_req_single_dash_separator_is_accepted() -> None:
    """`|-|-|-|-|-|` は GFM で正当。データ行と誤認して落とさない。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(
            repo, "docs/knowledge/a.md", "D19",
            ["| REQ-ID | 要件 | 検証手段 | 出典 | status |", "|-|-|-|-|-|", VALID_ROW],
            header=False,
        )
        code, out = check(repo)
        assert code == 0, out
    finally:
        shutil.rmtree(repo)


def test_req_unclosed_code_fence_is_error() -> None:
    """フェンスが開いたままだと、以降の REQ 表がフェンス内扱いで無検査になる。"""
    repo = new_repo()
    try:
        baseline(repo)
        path = repo / "docs/knowledge/a.md"
        path.write_text(path.read_text(encoding="utf-8") + "\n```sh\necho 未完\n",
                        encoding="utf-8")
        append_req_block(repo, "docs/knowledge/a.md", "D19", [VALID_ROW])
        code, out = check(repo)
        assert code == 1, out
        assert "コードフェンスが閉じられていない" in out, out
    finally:
        shutil.rmtree(repo)


def append_raw(repo: Path, rel: str, text: str) -> None:
    path = repo / rel
    path.write_text(path.read_text(encoding="utf-8") + text, encoding="utf-8")


def test_req_row_without_leading_pipe_is_error() -> None:
    """GFM は行頭パイプを省いても表になる。捨てると一意性検査から消えて重複が通る。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         [VALID_ROW, "REQ-D19-001 | 別の要件 | `cargo test` | ADR 0001 | Confirmed |"])
        code, out = check(repo)
        assert code == 1, out
        assert "表の行として読めない" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_unmarked_table_without_leading_pipe_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md",
                   "\nREQ-ID | 要件 | 検証手段 | 出典 | status\n"
                   "---|---|---|---|---\n"
                   "REQ-D19-001 | 何か | `cargo test` | ADR 0001 | Confirmed\n")
        code, out = check(repo)
        assert code == 1, out
        assert "マーカーの外にある" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_prose_mention_is_not_error() -> None:
    """地の文の REQ-ID 言及で落とすと、規約文も目標文書も書けなくなる。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19", [VALID_ROW])
        append_raw(repo, "docs/knowledge/a.md", "\n詳細は REQ-D19-001 を参照する。\n")
        code, out = check(repo)
        assert code == 0, f"地の文の言及で落ちた: {out}"
    finally:
        shutil.rmtree(repo)


def test_req_prose_with_pipe_is_not_error() -> None:
    """地の文が REQ-ID に言及し、コマンド例にパイプが混じるだけでは落とさない。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19", [VALID_ROW])
        append_raw(repo, "docs/knowledge/a.md",
                   "\nREQ-D19-001 の検証は `cargo test | tail -1` で行う。\n")
        code, out = check(repo)
        assert code == 0, f"地の文＋パイプで落ちた: {out}"
    finally:
        shutil.rmtree(repo)


def test_req_inline_code_mention_in_block_is_not_error() -> None:
    """ブロック内の注記がインラインコードで REQ-ID に触れるだけでは落とさない。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         [VALID_ROW, "`REQ-D19-001` は #123 で見直す予定。"])
        code, out = check(repo)
        assert code == 0, f"インラインコードの言及で落ちた: {out}"
    finally:
        shutil.rmtree(repo)


def test_req_traceability_table_is_not_error() -> None:
    """REQ-ID を右の列で参照するだけの表は REQ 表ではない（第 1 セルで判定する）。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19", [VALID_ROW])
        append_raw(repo, "docs/knowledge/a.md",
                   "\n| 実装 | 対応 REQ |\n|---|---|\n| build_portfolio | REQ-D19-001 |\n")
        code, out = check(repo)
        assert code == 0, f"トレーサビリティ表で落ちた: {out}"
    finally:
        shutil.rmtree(repo)


def test_req_block_in_blockquote_is_checked() -> None:
    """`> ` を付けるだけで全検査を素通りさせない（GitHub は引用内でも表として描く）。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md",
                   "\n> <!-- REQ:begin D19 -->\n"
                   "> | REQ-ID | 要件 | 検証手段 | 出典 | status |\n"
                   "> |---|---|---|---|---|\n"
                   "> | REQ-D19-001 | 何か | - | ADR 0001 | Confirmed |\n"
                   "> <!-- REQ:end D19 -->\n")
        code, out = check(repo)
        assert code == 1, out
        assert "検証手段が空のまま Confirmed" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_link_to_directory_is_accepted() -> None:
    """ディレクトリへの相対リンクも正当な Markdown（`scripts/` 等を引ける）。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         ["| REQ-D19-001 | 何か | `cargo test` "
                          "| [一次資料](../original-docs/) | Confirmed |"])
        code, out = check(repo)
        assert code == 0, f"ディレクトリリンクで落ちた: {out}"
    finally:
        shutil.rmtree(repo)


def test_req_nested_fence_is_not_closed_early() -> None:
    """```` の中の ``` で閉じると、見本が実データに化けて偽陽性になる。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md",
                   "\n````markdown\n```\n| REQ-D19-999 | 見本 | | | Bogus |\n```\n````\n")
        code, out = check(repo)
        assert code == 0, f"入れ子フェンスで落ちた: {out}"
    finally:
        shutil.rmtree(repo)


def test_req_link_with_title_is_checked() -> None:
    """タイトル付きリンクを取りこぼすと、それだけで実在検査を迂回できる。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         ['| REQ-D19-001 | 何か | `cargo test` '
                          '| [ADR 0099](../original-docs/0099-nope.md "題") | Confirmed |'])
        code, out = check(repo)
        assert code == 1, out
        assert "リンク先が実在しない" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_link_inside_inline_code_is_ignored() -> None:
    """検証手段はコマンドを書く列。コード内のリンク様文字列は検査しない。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         ["| REQ-D19-001 | 何か | `grep '[x](nope.md)' file` "
                          "| ADR 0001 | Confirmed |"])
        code, out = check(repo)
        assert code == 0, f"コード内のリンク様文字列で落ちた: {out}"
    finally:
        shutil.rmtree(repo)


def test_req_empty_requirement_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         ["| REQ-D19-001 | - | `cargo test` | ADR 0001 | Confirmed |"])
        code, out = check(repo)
        assert code == 1, out
        assert "要件が空" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_empty_origin_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         ["| REQ-D19-001 | 何か | `cargo test` | TBD | Confirmed |"])
        code, out = check(repo)
        assert code == 1, out
        assert "出典が空" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_orphan_end_marker_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md", "\n<!-- REQ:end D19 -->\n")
        code, out = check(repo)
        assert code == 1, out
        assert "begin の無い" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_retired_status_is_accepted() -> None:
    """番号を再利用しないための唯一の逃げ道なので、通る側も固定する。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         ["| REQ-D19-001 | かつての要件 | - | ADR 0001 | Retired |"])
        code, out = check(repo)
        assert code == 0, out
    finally:
        shutil.rmtree(repo)


def test_req_undefined_block_class_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(repo, "docs/knowledge/a.md", "D99",
                         ["| REQ-D99-001 | 何か | `cargo test` | ADR 0001 | Confirmed |"])
        code, out = check(repo)
        assert code == 1, out
        assert "定義が無い" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_separator_column_count_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(
            repo, "docs/knowledge/a.md", "D19",
            ["| REQ-ID | 要件 | 検証手段 | 出典 | status |", "|---|---|", VALID_ROW],
            header=False,
        )
        code, out = check(repo)
        assert code == 1, out
        assert "区切り行が" in out, out
    finally:
        shutil.rmtree(repo)


# --- (9) 本文の相対リンク（#604）。REQ 表の外は無検査だった ---


def test_body_broken_link_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md", "\n[出典](../original-docs/9999-nope.md)\n")
        code, out = check(repo)
        assert code == 1, out
        assert "本文（" in out and "リンク先が実在しない" in out, out
    finally:
        shutil.rmtree(repo)


def test_body_link_to_existing_file_passes() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md", "\n[出典](../original-docs/0001-first.md)\n")
        code, out = check(repo)
        assert code == 0, out
    finally:
        shutil.rmtree(repo)


def test_body_link_to_directory_is_accepted() -> None:
    """ディレクトリへの相対リンクは正当（sources の is_file とは意図的に非対称）。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md", "\n[一次資料](../original-docs/)\n")
        code, out = check(repo)
        assert code == 0, out
    finally:
        shutil.rmtree(repo)


def test_body_absolute_link_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md", "\n[外](/etc/hosts)\n")
        code, out = check(repo)
        assert code == 1, out
        assert "本文（" in out and "リンクは文書からの相対パスで書く" in out, out
    finally:
        shutil.rmtree(repo)


def test_body_link_outside_repo_is_error() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md", "\n[外](../../../etc/hosts)\n")
        code, out = check(repo)
        assert code == 1, out
        assert "本文（" in out and "リンクがリポジトリ外を指している" in out, out
    finally:
        shutil.rmtree(repo)


def test_body_link_in_code_fence_is_ignored() -> None:
    """規約の見本（テンプレート）を実データとして検査しない。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(
            repo, "docs/knowledge/a.md",
            "\n```md\n[見本](../original-docs/0NNN-....md)\n```\n",
        )
        code, out = check(repo)
        assert code == 0, out
    finally:
        shutil.rmtree(repo)


def test_body_link_inside_inline_code_is_ignored() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md", "\n`grep '[x](nope.md)' file` を実行する\n")
        code, out = check(repo)
        assert code == 0, out
    finally:
        shutil.rmtree(repo)


def test_body_external_link_is_skipped() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md", "\n[外部](https://example.invalid/x)\n")
        code, out = check(repo)
        assert code == 0, out
    finally:
        shutil.rmtree(repo)


def test_broken_link_in_req_table_is_reported_once() -> None:
    """REQ 表の行は本文にも含まれる。台帳を共有して同じリンクを二重報告しない。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(
            repo, "docs/knowledge/a.md", "D19",
            ["| REQ-D19-001 | 要件 | `cargo test` | [ADR](../original-docs/9999-nope.md) |"
             " Confirmed |"],
        )
        code, out = check(repo)
        assert code == 1, out
        assert out.count("9999-nope.md") == 1, f"同じリンクが二重に報告された:\n{out}"
    finally:
        shutil.rmtree(repo)


# --- (10) 割当索引と実ファイルの突合（#604） ---


def test_index_missing_row_is_error() -> None:
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_registry(repo, sha, docs=[])
        code, out = check(repo)
        assert code == 1, out
        assert "割当索引に knowledge/a.md の行が無い" in out, out
    finally:
        shutil.rmtree(repo)


def test_index_ghost_row_is_error() -> None:
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_registry(repo, sha,
                       docs=[("knowledge/a.md", ["D19"]), ("knowledge/ghost.md", ["D19"])])
        code, out = check(repo)
        assert code == 1, out
        assert "割当索引の knowledge/ghost.md は対応する検査対象の文書が無い" in out, out
    finally:
        shutil.rmtree(repo)


def test_index_value_mismatch_is_error() -> None:
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_registry(repo, sha, docs=[("knowledge/a.md", ["D08"])])
        code, out = check(repo)
        assert code == 1, out
        assert "割当索引の knowledge/a.md が frontmatter と一致しない" in out, out
    finally:
        shutil.rmtree(repo)


def test_index_class_order_mismatch_is_error() -> None:
    """第 1 要素が主クラスなので順序違いも不一致。集計数では検出できない経路。"""
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_doc(repo, "docs/knowledge/a.md", ["D19", "D22"],
                  ["docs/original-docs/0001-first.md"], sha)
        write_registry(repo, sha, d19=1, d22=1, docs=[("knowledge/a.md", ["D22", "D19"])])
        code, out = check(repo)
        assert code == 1, out
        assert "索引=[D22, D19] / 実際=[D19, D22]" in out, out
    finally:
        shutil.rmtree(repo)


def test_index_swap_between_docs_is_error() -> None:
    """2 文書間でクラスを交換しても集計数は変わらない。索引だけが気づける。"""
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_doc(repo, "docs/knowledge/b.md", ["D22"],
                  ["docs/original-docs/0001-first.md"], sha)
        write_registry(repo, sha, d19=1, d22=1,
                       docs=[("knowledge/a.md", ["D22"]), ("knowledge/b.md", ["D19"])])
        code, out = check(repo)
        assert code == 1, out
        assert "割当索引の knowledge/a.md が frontmatter と一致しない" in out, out
        assert "割当索引の knowledge/b.md が frontmatter と一致しない" in out, out
    finally:
        shutil.rmtree(repo)


def test_index_marker_missing_is_fatal_even_with_warn_only() -> None:
    """逃げ道（--warn-only）でも落ちること。表の範囲を切り出せず検査が成立しないため。"""
    repo = new_repo()
    try:
        baseline(repo)
        registry = repo / "docs/knowledge/doc-classes.md"
        text = registry.read_text(encoding="utf-8")
        registry.write_text(text.replace("<!-- doc-classes-index:begin -->", ""), encoding="utf-8")
        code, out = check(repo, "--warn-only")
        assert code == 1, out
        assert "doc-classes-index:begin" in out, out
    finally:
        shutil.rmtree(repo)


def test_index_marker_missing_is_fatal() -> None:
    """マーカーを消して検査を素通りさせる経路を塞ぐ（fail-closed）。"""
    repo = new_repo()
    try:
        sha = baseline(repo)
        registry = repo / "docs/knowledge/doc-classes.md"
        text = registry.read_text(encoding="utf-8")
        assert "<!-- doc-classes-index:begin -->" in text
        registry.write_text(text.replace("<!-- doc-classes-index:begin -->", ""), encoding="utf-8")
        code, out = check(repo)
        assert code == 1, out
        assert "doc-classes-index:begin" in out, out
        assert sha  # baseline の SHA は使わないが、pin 済みであることを明示
    finally:
        shutil.rmtree(repo)


# --- 1 巡目レビューで見つかった穴の回帰（#608） ---


def test_body_link_after_stray_backtick_is_still_checked() -> None:
    """散文中の単独バッククォートがリンクを飲み込まない（インラインコードは改行を跨がない）。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(
            repo, "docs/knowledge/a.md",
            "\n散文に ` が 1 つある。\n\n[壊れ](../original-docs/9999-nope.md)\n\n"
            "そして `cargo test` を実行。\n",
        )
        code, out = check(repo)
        assert code == 1, out
        assert "本文（" in out and "リンク先が実在しない" in out, out
    finally:
        shutil.rmtree(repo)


def test_body_link_in_tilde_fence_is_ignored() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md",
                   "\n~~~md\n[見本](../original-docs/0NNN-....md)\n~~~\n")
        code, out = check(repo)
        assert code == 0, out
    finally:
        shutil.rmtree(repo)


def test_body_link_in_quoted_fence_is_ignored() -> None:
    """blockquote の中のフェンスも閉じる（`> ` を付けるだけで素通りさせない）。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md",
                   "\n> ```\n> [見本](../original-docs/0NNN-....md)\n> ```\n")
        code, out = check(repo)
        assert code == 0, out
    finally:
        shutil.rmtree(repo)


def test_body_same_broken_link_twice_is_reported_once() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(
            repo, "docs/knowledge/a.md",
            "\n[1](../original-docs/9999-nope.md) と [2](../original-docs/9999-nope.md#節)\n",
        )
        code, out = check(repo)
        assert code == 1, out
        assert out.count("9999-nope.md") == 1, f"アンカー違いで二重報告された:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_body_other_uri_schemes_are_skipped() -> None:
    """http/https 以外のスキームと protocol-relative を実在検査しない。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md",
                   "\n[f](ftp://example.invalid/x) [t](tel:0120) [p](//example.invalid/a)\n")
        code, out = check(repo)
        assert code == 0, out
    finally:
        shutil.rmtree(repo)


def test_body_image_link_is_checked() -> None:
    """画像も実在検査の対象（図が消えても気づけないと索引文書が腐る）。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md", "\n![図](diagrams/nope.svg)\n")
        code, out = check(repo)
        assert code == 1, out
        assert "本文（" in out and "リンク先が実在しない" in out, out
    finally:
        shutil.rmtree(repo)


def _case_insensitive_fs(repo: Path) -> bool:
    """fixture を置いた FS が大文字小文字を区別しないか（macOS か Linux か）。"""
    return (repo / "docs/original-docs/0001-FIRST.md").exists()


def test_body_link_case_mismatch_is_error() -> None:
    """大文字小文字違い。macOS では exists() が通るので専用の判定が要る。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md", "\n[大小](../original-docs/0001-FIRST.md)\n")
        code, out = check(repo)
        assert code == 1, out
        if _case_insensitive_fs(repo):
            # ここを OR で緩めると、case-sensitive な CI では新分岐が一度も実行されず
            # case_exact を丸ごと消しても緑のままになる。
            assert "大文字小文字が実ファイルと違う" in out, out
        else:
            assert "実在しない" in out, out
    finally:
        shutil.rmtree(repo)


def test_body_link_directory_case_mismatch_is_error() -> None:
    """ディレクトリ成分の大小違いも見る（最終成分だけ照合すると素通りする）。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md", "\n[大小](../Original-docs/0001-first.md)\n")
        code, out = check(repo)
        assert code == 1, out
        if _case_insensitive_fs(repo):
            assert "大文字小文字が実ファイルと違う" in out, out
        else:
            assert "実在しない" in out, out
    finally:
        shutil.rmtree(repo)


def test_index_malformed_row_is_error() -> None:
    repo = new_repo()
    try:
        sha = baseline(repo)
        registry = repo / "docs/knowledge/doc-classes.md"
        text = registry.read_text(encoding="utf-8")
        registry.write_text(
            text.replace("<!-- doc-classes-index:end -->",
                         "| 列が 1 つしかない |\n<!-- doc-classes-index:end -->"),
            encoding="utf-8",
        )
        code, out = check(repo)
        assert code == 1, out
        assert "割当索引の書式が崩れている行がある" in out, out
        assert sha
    finally:
        shutil.rmtree(repo)


def test_index_duplicate_row_is_error() -> None:
    """後勝ちで上書きすると、片方が実態とズレていても無言で通る。"""
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_registry(repo, sha,
                       docs=[("knowledge/a.md", ["D19"]), ("knowledge/a.md", ["D19"])])
        code, out = check(repo)
        assert code == 1, out
        assert "割当索引に knowledge/a.md の行が 2 つある" in out, out
    finally:
        shutil.rmtree(repo)


def test_index_row_for_doc_without_doc_class_says_why() -> None:
    """ファイルはあるが doc_class を読めない場合に「文書が無い」と誤誘導しない。"""
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
        assert "doc_class を読めない" in out, out
    finally:
        shutil.rmtree(repo)


def test_readme_body_link_is_checked() -> None:
    """全検査から外している README も、リンクだけは見る（規約の正本を無検査にしない）。"""
    repo = new_repo()
    try:
        baseline(repo)
        (repo / "docs/knowledge/README.md").write_text(
            "# 規約\n\n```yaml\ndistilled_from_sha: \"<short-sha>\"\n```\n"
            "\n[壊れ](../original-docs/9999-nope.md)\n",
            encoding="utf-8",
        )
        code, out = check(repo)
        assert code == 1, out
        assert "docs/knowledge/README.md: 本文（" in out and "実在しない" in out, out
    finally:
        shutil.rmtree(repo)


def test_readme_template_in_fence_is_ignored() -> None:
    repo = new_repo()
    try:
        baseline(repo)
        (repo / "docs/knowledge/README.md").write_text(
            "# 規約\n\n```md\n[見本](../original-docs/0NNN-....md)\n```\n",
            encoding="utf-8",
        )
        code, out = check(repo)
        assert code == 0, out
    finally:
        shutil.rmtree(repo)


def _reported_line(out: str) -> int:
    matched = re.search(r"本文（(\d+) 行目）", out)
    assert matched, out
    return int(matched.group(1))


def test_body_link_line_number_matches_file() -> None:
    """報告した行番号の実ファイル行に、そのリンクがあること（frontmatter 分のオフセット）。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md",
                   "\n埋草。\n\n[壊れ](../original-docs/9999-nope.md)\n")
        code, out = check(repo)
        assert code == 1, out
        lineno = _reported_line(out)
        lines = (repo / "docs/knowledge/a.md").read_text(encoding="utf-8").splitlines()
        assert "9999-nope.md" in lines[lineno - 1], f"{lineno} 行目は {lines[lineno - 1]!r}"
    finally:
        shutil.rmtree(repo)


def test_readme_link_line_number_matches_file() -> None:
    """frontmatter を持たない文書ではオフセット 0（README）。"""
    repo = new_repo()
    try:
        baseline(repo)
        (repo / "docs/knowledge/README.md").write_text(
            "# 規約\n\n埋草。\n\n[壊れ](../original-docs/9999-nope.md)\n", encoding="utf-8"
        )
        code, out = check(repo)
        assert code == 1, out
        lineno = _reported_line(out)
        lines = (repo / "docs/knowledge/README.md").read_text(encoding="utf-8").splitlines()
        assert "9999-nope.md" in lines[lineno - 1], f"{lineno} 行目は {lines[lineno - 1]!r}"
    finally:
        shutil.rmtree(repo)


def test_body_link_with_label_across_lines_is_checked() -> None:
    """ラベルが改行を跨ぐリンクも拾う（行単位で切ると黙って無検査になる）。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(repo, "docs/knowledge/a.md",
                   "\n[長いラベルの\n続き](../original-docs/9999-nope.md)\n")
        code, out = check(repo)
        assert code == 1, out
        assert "9999-nope.md" in out, out
    finally:
        shutil.rmtree(repo)


def test_claude_md_body_link_is_checked() -> None:
    """毎セッション読まれる CLAUDE.md も（ディレクトリ走査の外だが）リンクだけは見る。"""
    repo = new_repo()
    try:
        baseline(repo)
        (repo / "CLAUDE.md").write_text(
            "# 運用指示\n\n[用語集](docs/knowledge/nope.md)\n", encoding="utf-8"
        )
        code, out = check(repo)
        assert code == 1, out
        assert "CLAUDE.md: 本文（" in out and "実在しない" in out, out
    finally:
        shutil.rmtree(repo)


def test_line_number_is_correct_after_inline_code() -> None:
    """インラインコードの手前があると位置がずれる（長さ保存で置換していないと過少になる）。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_raw(
            repo, "docs/knowledge/a.md",
            "\n`とても長いインラインコードの行`\n\n`もう一つ長いインラインコード`\n\n"
            "[壊れ](../original-docs/9999-nope.md)\n",
        )
        code, out = check(repo)
        assert code == 1, out
        lineno = _reported_line(out)
        lines = (repo / "docs/knowledge/a.md").read_text(encoding="utf-8").splitlines()
        assert "9999-nope.md" in lines[lineno - 1], f"{lineno} 行目は {lines[lineno - 1]!r}"
    finally:
        shutil.rmtree(repo)


def test_unclosed_fence_in_readme_is_error() -> None:
    """README / CLAUDE.md は REQ 走査を通らないので、閉じ忘れを自前で報告する必要がある。"""
    repo = new_repo()
    try:
        baseline(repo)
        (repo / "docs/knowledge/README.md").write_text(
            "# 規約\n\n```md\n[見本](../original-docs/0NNN-....md)\n", encoding="utf-8"
        )
        code, out = check(repo)
        assert code == 1, out
        assert "コードフェンスが閉じられていない" in out, out
    finally:
        shutil.rmtree(repo)


def test_document_without_frontmatter_still_gets_link_check() -> None:
    """frontmatter が無い文書でも本文リンクは見る（continue で丸ごと飛ばさない）。"""
    repo = new_repo()
    try:
        baseline(repo)
        (repo / "docs/knowledge/b.md").write_text(
            "# b\n\n[壊れ](../original-docs/9999-nope.md)\n", encoding="utf-8"
        )
        code, out = check(repo)
        assert code == 1, out
        assert "docs/knowledge/b.md: frontmatter が無い" in out, out
        assert "docs/knowledge/b.md: 本文（" in out and "実在しない" in out, out
    finally:
        shutil.rmtree(repo)


def test_req_cell_link_is_backstop_for_body_scan() -> None:
    """セルを跨ぐバッククォートで本文走査が飲み込むリンクを、REQ のセル単位走査が拾う。

    本文は 1 文字列として走査するので、`検証手段` セルの開きバッククォートと `出典` セルの
    閉じバッククォートが対になると、その間のリンクがインラインコード扱いで消える。
    セル単位の走査はこの backstop で、消すとこの経路が丸ごと無検査になる。
    """
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(
            repo, "docs/knowledge/a.md", "D19",
            ["| REQ-D19-001 | 要件 | `cmd | [ADR](../original-docs/9999-nope.md)` | Confirmed |"],
        )
        code, out = check(repo)
        assert code == 1, out
        assert "REQ-D19-001 のリンク先が実在しない" in out, out
    finally:
        shutil.rmtree(repo)


# --- 検査 11: REQ 表の出典 ⊆ frontmatter の sources（#597 / ADR 0083） ---

FIRST_ADR = "docs/original-docs/0001-first.md"
SECOND_ADR = "docs/original-docs/0002-second.md"


def add_adr(repo: Path, name: str) -> str:
    """一次資料を 1 本足す。返り値はリポジトリ相対パス（sources に書く形式）。"""
    rel = f"docs/original-docs/{name}"
    (repo / rel).write_text(
        f"# {name.split('-')[0]}. テスト用の決定\n\n## 決定\n\nこうする。\n", encoding="utf-8"
    )
    return rel


def repin(repo: Path, entries: "list[tuple[str, list[str], list[str]]]", **registry_kw) -> str:
    """`entries`（rel, classes, sources）とレジストリを書き、baseline と同じ 2 段コミットで
    frontmatter の sha を確定させる。

    新しい一次資料を足したあとに使う。1 段だと「一次資料の最終内容変更」が
    distilled_from_sha の子孫になり、狙いと無関係な STALE で落ちる。

    第 2 引数を `docs` にしないのは、`write_registry(docs=...)` へ素通しする
    `registry_kw` と名前が衝突するため（`repin(..., docs=[...])` が TypeError になる）。
    """
    for rel, classes, sources in entries:
        write_doc(repo, rel, classes, sources, "HEAD")
    write_registry(repo, "HEAD", **registry_kw)
    sha = commit_all(repo, "add primary docs")
    for rel, classes, sources in entries:
        write_doc(repo, rel, classes, sources, sha)
    write_registry(repo, sha, **registry_kw)
    return commit_all(repo, "pin sha")


def origin_row(origin: str, req_id: str = "REQ-D19-001") -> str:  # noqa: D401
    return f"| {req_id} | 何かを満たす | `cargo test` で固定 | {origin} | Confirmed |"


def test_req_origin_in_sources_passes() -> None:
    """出典が名指しした ADR が sources にも載っていれば通る。"""
    repo = new_repo()
    try:
        baseline(repo)
        add_adr(repo, "0002-second.md")
        repin(repo, [("docs/knowledge/a.md", ["D19"], [FIRST_ADR, SECOND_ADR])])
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         [origin_row("[ADR 0002](../original-docs/0002-second.md)")])
        code, out = check(repo)
        assert code == 0, f"出典が sources にあるのに落ちた: {out}"
    finally:
        shutil.rmtree(repo)


def test_req_origin_missing_from_sources_is_error() -> None:
    """出典で名指ししたのに sources に無ければ落とす（sources から消せば stale も消える穴）。"""
    repo = new_repo()
    try:
        baseline(repo)
        add_adr(repo, "0002-second.md")
        # a.md だけが出典で 0002 を名指ししていて sources に無い、という状況を作る。
        repin(
            repo,
            [
                ("docs/knowledge/a.md", ["D19"], [FIRST_ADR]),
                ("docs/knowledge/b.md", ["D19"], [FIRST_ADR, SECOND_ADR]),
            ],
            d19=2,
            docs=[("knowledge/a.md", ["D19"]), ("knowledge/b.md", ["D19"])],
        )
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         [origin_row("[ADR 0002](../original-docs/0002-second.md)")])
        code, out = check(repo)
        assert code == 1, out
        assert "sources に無い" in out and SECOND_ADR in out, out
    finally:
        shutil.rmtree(repo)


def test_req_origin_external_url_is_skipped() -> None:
    """出典が GitHub issue の絶対 URL だけなら対象外（一次資料ファイルではない）。"""
    repo = new_repo()
    try:
        baseline(repo)
        append_req_block(
            repo, "docs/knowledge/a.md", "D19",
            [origin_row("[#350](https://github.com/taito-station/paddock/issues/350)")],
        )
        code, out = check(repo)
        assert code == 0, f"外部 URL の出典で落ちた: {out}"
    finally:
        shutil.rmtree(repo)


def test_req_origin_issue_derived_primary_doc_is_checked() -> None:
    """検査 11 の対象は `docs/original-docs/` 配下**全体**（4 桁 ADR に限らない）。

    QA Q4 の意図的なスコープ判断を pin する。実装が ADR 限定へ退行しても、
    このテストが無いと 4 桁 ADR しか使わない他の 3 本は全部通ってしまう。
    """
    repo = new_repo()
    try:
        baseline(repo)
        primary = add_adr(repo, "382-live-server-now.md")
        # b.md が 382 を sources に持ち、a.md の出典だけが未収載という状況を作る。
        repin(
            repo,
            [
                ("docs/knowledge/a.md", ["D19"], [FIRST_ADR]),
                ("docs/knowledge/b.md", ["D19"], [FIRST_ADR, primary]),
            ],
            d19=2,
            docs=[("knowledge/a.md", ["D19"]), ("knowledge/b.md", ["D19"])],
        )
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         [origin_row("[#382](../original-docs/382-live-server-now.md)")])
        code, out = check(repo)
        assert code == 1, out
        assert "sources に無い" in out and primary in out, out
    finally:
        shutil.rmtree(repo)


def test_req_origin_sibling_doc_link_is_skipped() -> None:
    """出典が兄弟の knowledge を指すときは対象外（蒸留元ではないので sources に載せない）。"""
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_doc(repo, "docs/knowledge/b.md", ["D19"], [FIRST_ADR], sha)
        write_registry(repo, sha, d19=2,
                       docs=[("knowledge/a.md", ["D19"]), ("knowledge/b.md", ["D19"])])
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         [origin_row("[b の定義](b.md)")])
        code, out = check(repo)
        assert code == 0, f"兄弟文書へのリンクで落ちた: {out}"
    finally:
        shutil.rmtree(repo)


# --- 2 巡目レビューの変異テストで「消しても緑のまま」と判明した契約の pin ---
# 以下は実装から該当行を削っても 127 ケースが全通過してしまった分岐。挙動そのものは
# 正しかったが、テストが 1 本も見ていなかったので退行を検出できない状態だった。


def test_req_origin_same_missing_source_reported_once() -> None:
    """同じ未収載出典を複数の REQ が挙げても、報告は文書内で 1 回に畳む。"""
    repo = new_repo()
    try:
        baseline(repo)
        add_adr(repo, "0002-second.md")
        repin(
            repo,
            [
                ("docs/knowledge/a.md", ["D19"], [FIRST_ADR]),
                ("docs/knowledge/b.md", ["D19"], [FIRST_ADR, SECOND_ADR]),
            ],
            d19=2,
            docs=[("knowledge/a.md", ["D19"]), ("knowledge/b.md", ["D19"])],
        )
        link = "[ADR 0002](../original-docs/0002-second.md)"
        append_req_block(repo, "docs/knowledge/a.md", "D19",
                         [origin_row(link, "REQ-D19-001"), origin_row(link, "REQ-D19-002")])
        code, out = check(repo)
        assert code == 1, out
        # 件数まで assert しないと dedup を消しても緑のまま（変異テストで実証済み）。
        assert out.count("sources に無い") == 1, f"同じ出典が二重に報告された:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_req_verification_link_is_not_checked_against_sources() -> None:
    """突合の対象は `出典` 列だけ。`検証手段` 列は測り方であって蒸留元ではない。"""
    repo = new_repo()
    try:
        baseline(repo)
        add_adr(repo, "0002-second.md")
        repin(
            repo,
            [
                ("docs/knowledge/a.md", ["D19"], [FIRST_ADR]),
                ("docs/knowledge/b.md", ["D19"], [FIRST_ADR, SECOND_ADR]),
            ],
            d19=2,
            docs=[("knowledge/a.md", ["D19"]), ("knowledge/b.md", ["D19"])],
        )
        append_req_block(
            repo, "docs/knowledge/a.md", "D19",
            ["| REQ-D19-001 | 何かを満たす | "
             "[ADR 0002](../original-docs/0002-second.md) の手順で再実行 | ADR 0001 | Confirmed |"],
        )
        code, out = check(repo)
        assert code == 0, f"検証手段の列を sources と突合した: {out}"
    finally:
        shutil.rmtree(repo)


def test_noncanonical_source_is_error() -> None:
    """`./docs/...` のような非正規形の sources を弾く。

    非正規形は実在検査を通るのに stale 判定の突合から静かに外れる（path_status が
    `git show --name-status` の出力と終点一致で突き合わせるため）。形式を 1 つに
    強制することで、4 / 6 / 11 が同じ文字列を見ることを保証する。
    """
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_doc(repo, "docs/knowledge/a.md", ["D19"], ["./" + FIRST_ADR], sha)
        code, out = check(repo)
        assert code == 1, out
        assert "sources は正規形で書く" in out, out
    finally:
        shutil.rmtree(repo)


def test_source_case_mismatch_is_error() -> None:
    """sources の大文字小文字違いを弾く。

    macOS(APFS) では is_file() が通ってしまい、Linux の CI だけが落ちる。しかも手元では
    stale 判定が「履歴を辿れず」の warning に退化する（`./` 形式と同じ silent-green）。
    """
    repo = new_repo()
    try:
        sha = baseline(repo)
        write_doc(repo, "docs/knowledge/a.md", ["D19"],
                  ["docs/original-docs/0001-FIRST.md"], sha)
        code, out = check(repo)
        assert code == 1, out
        if (repo / "docs/original-docs/0001-FIRST.md").exists():
            # 大文字小文字を区別しない FS（macOS）。区別する FS では実在しない側に落ちる。
            assert "sources の大文字小文字が実ファイルと違う" in out, out
        else:
            assert "sources のパスが実在しない" in out, out
    finally:
        shutil.rmtree(repo)


# --- マージコミットに対する stale 判定（#615 (a) / ADR 0084） ---
#
# `path_status` は `git show --format= --name-status -M100% <sha>` を使う。git はマージに対して
# **既定で combined diff（`--cc`）**を出し、`--cc` は「**全ての親と異なる**パス」を列挙する
# ——これは evil merge（マージ自身だけが内容を変える形）の定義そのもの。したがって
# evil merge は検出できている。**この `--cc` 依存は契約**なので、`--first-parent` の追加や
# `git diff-tree` への置換で壊れることを下のテストで固定する。


def git_allow_fail(repo: Path, *args: str) -> "subprocess.CompletedProcess[str]":
    """`run_git` と違い失敗を許す。コンフリクトする `git merge` は 1 を返すため。"""
    return subprocess.run(["git", *args], cwd=repo, capture_output=True, text=True)


PIN_TOOLCHAIN_RIVAL = "f" * 40
PIN_TOOLCHAIN_THIRD = "1" * 40


def test_evil_merge_is_detected_as_content_change() -> None:
    """マージ自身だけが内容を変える evil merge を stale 判定が見落とさない。

    **両親の変更を「免除対象」（ピン更新のみ）にするのが要点。** そうしないと、マージが
    不可視になっても親側の変更が STALE を出してしまい、テストが何も識別しない
    （実際に一度そう書いてしまい、`path_status` がマージで `(None, None)` を返す変異を
    注入しても緑のままだった）。免除対象で挟めば、**マージが見えなくなった瞬間に緑へ転ぶ**。

    `path_status` が `git show` の combined diff（`--cc`）に依存している事実の契約テスト。
    `git diff-tree`（`-c` 無し）への置換など、マージで無出力になる変更が入ると落ちる。
    """
    repo = new_repo()
    try:
        workflow_baseline(repo)
        pinned = run_git(repo, "rev-parse", "--short", "HEAD")
        base = run_git(repo, "rev-parse", "--abbrev-ref", "HEAD")

        # 両側が「同じピンを別の hex へ」動かす＝どちらも免除対象、かつ必ずコンフリクトする。
        run_git(repo, "checkout", "-q", "-b", "side")
        write_workflow(repo, toolchain=PIN_TOOLCHAIN_NEW)
        commit_all(repo, "side: ピン更新のみ")
        run_git(repo, "checkout", "-q", base)
        write_workflow(repo, toolchain=PIN_TOOLCHAIN_RIVAL)
        commit_all(repo, "base: ピン更新のみ")

        conflicted = git_allow_fail(repo, "merge", "side")
        assert conflicted.returncode != 0, "前提: 同じ行を両側で変えたのでコンフリクトする"
        # 解決のついでにステップ名を変える＝**どちらの親にも無い内容変更**。
        write_workflow(repo, toolchain=PIN_TOOLCHAIN_NEW, comment="解決時に書き換えた注記")
        merge = commit_all(repo, "evil merge")
        assert len(run_git(repo, "rev-parse", f"{merge}^@").split()) == 2, "前提: 2 親のマージ"

        # a.md の distill は両親より前に固定したまま。
        code, out = check(repo)
        assert code == 1, (
            f"evil merge が見落とされた（git show の combined diff を失っていないか）:\n{out}"
        )
        assert "STALE" in out and merge[:7] in out, out
        assert pinned  # 免除対象で挟んでいることの明示
    finally:
        shutil.rmtree(repo)


def test_pin_only_merge_is_not_stale() -> None:
    """上のテストの対照群。マージ自身が内容を変えても、それが**免除対象**なら STALE にならない。

    これが無いと `test_evil_merge_is_detected_as_content_change` の exit 1 が
    「マージだから」なのか「内容が変わったから」なのか区別できない。

    **解決に第 3 の hex を書くのが要点。** 片親の hex をそのまま採ると対象パスについて
    その親と TREESAME になり、`git log` がマージを列挙しないので `path_status` も
    免除分岐も**一度も呼ばれない**——「マージが免除された」ではなく「マージが最初から
    見えない」ことを確かめるだけの空テストになる（1 巡目レビューで実測・指摘された）。
    第 3 の hex なら全親と異なるのでマージが走査に載り、そのうえで免除が効くことを見られる。
    """
    repo = new_repo()
    try:
        workflow_baseline(repo)
        base = run_git(repo, "rev-parse", "--abbrev-ref", "HEAD")
        run_git(repo, "checkout", "-q", "-b", "side")
        write_workflow(repo, toolchain=PIN_TOOLCHAIN_NEW)
        commit_all(repo, "side: ピン更新のみ")
        run_git(repo, "checkout", "-q", base)
        write_workflow(repo, toolchain=PIN_TOOLCHAIN_RIVAL)
        commit_all(repo, "base: ピン更新のみ")

        conflicted = git_allow_fail(repo, "merge", "side")
        assert conflicted.returncode != 0, "前提: 同じ行を両側で変えたのでコンフリクトする"
        # どちらの親とも違う hex で解決する＝マージ自身が内容を変えるが、免除対象のまま。
        write_workflow(repo, toolchain=PIN_TOOLCHAIN_THIRD)
        merge = commit_all(repo, "merge: 第 3 の hex で解決（ピン更新のみ）")
        assert len(run_git(repo, "rev-parse", f"{merge}^@").split()) == 2, "前提: 2 親のマージ"
        assert merge[:7] in run_git(repo, "log", "--format=%h", "--", WORKFLOW_REL), (
            "前提: マージが走査に載っていること（載らないと免除分岐を一度も通らない）"
        )

        code, out = check(repo)
        assert code == 0, f"ピン更新だけのマージで STALE になった:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_merge_taking_one_side_is_attributed_to_ancestor() -> None:
    """片親の内容をそのまま採るマージは、マージではなく**祖先コミット**に帰属する。

    こちらは `git log` の TREESAME 単純化がマージを飛ばすのが正しい——その内容を作った
    コミットが祖先に実在するため。evil merge との対比で固定する。
    """
    repo = new_repo()
    try:
        baseline(repo)
        base = run_git(repo, "rev-parse", "--abbrev-ref", "HEAD")
        run_git(repo, "checkout", "-q", "-b", "side")
        (repo / FIRST_ADR).write_text("# 0001\n\nside が作った内容\n", encoding="utf-8")
        side = commit_all(repo, "side change")
        run_git(repo, "checkout", "-q", base)
        (repo / "unrelated.md").write_text("x\n", encoding="utf-8")
        first_parent = commit_all(repo, "unrelated")
        run_git(repo, "merge", "-q", "side", "-m", "merge taking side")
        merge = run_git(repo, "rev-parse", "--short", "HEAD")
        write_doc(repo, "docs/knowledge/a.md", ["D19"], [FIRST_ADR], first_parent)
        write_registry(repo, first_parent)
        code, out = check(repo)
        assert code == 1, out
        # 報告されるのはマージではなく side のコミット。
        assert side[:7] in out, f"祖先ではなくマージに帰属した:\n{out}"
        assert merge[:7] not in out, f"マージに帰属した:\n{out}"
    finally:
        shutil.rmtree(repo)


def test_rename_source_commit_is_skipped_not_attributed() -> None:
    """パスを**リネーム元としてしか含まない**コミットを「内容変更」と誤認しない。

    `git log -- <path>` は純粋リネームのコミットを列挙するが、`git show --name-status` の
    出力は `R100 <path> <新パス>` で**終点が新パス**なので `path_status` の終点一致
    （`parts[-1] == path`）が外れて `(None, None)` になる。`scan_last_content_change` は
    そこを `continue` で飛ばす——**この `continue` は load-bearing で、`return sha` に
    変えると偽の STALE が出る**（ADR 0084 実測 4）。

    構成（**両側と解決を免除対象にするのが要点**。そうしないと走査がリネーム地点へ届かない）:
    `c1` が frontmatter 付きの一次資料 `src` を作る / mainline が `src` を別名へ**純粋リネーム** /
    side は `src` の frontmatter だけ動かす（例外 1b で免除）/ マージは side を第 1 親にして
    `src` を残し frontmatter だけ動かす（免除）。走査は マージ → side → **mainline のリネーム
    （ここで status is None）** → `c1` と進み、正しい答えは `c1`。
    """
    repo = new_repo()
    try:
        pre = baseline(repo)  # c1 の 1 つ前。ここに distill を固定して「答えが c1」を正で見る
        base = run_git(repo, "rev-parse", "--abbrev-ref", "HEAD")
        src = "docs/original-docs/0005-with-frontmatter.md"
        moved = "docs/original-docs/0009-moved.md"

        def write_src(updated: str) -> None:
            (repo / src).parent.mkdir(parents=True, exist_ok=True)
            (repo / src).write_text(
                f'---\nstatus: Confirmed\nkind: knowledge\nupdated: "{updated}"\n---\n\n'
                "# 0005. 一次資料\n\n本文（この巡では一切変えない）。\n",
                encoding="utf-8",
            )

        write_src("2026-01-01")
        c1 = commit_all(repo, "c1: 一次資料を作る")

        run_git(repo, "checkout", "-q", "-b", "side")
        write_src("2026-01-02")  # frontmatter だけ＝免除対象
        commit_all(repo, "side: updated だけ動かす")

        run_git(repo, "checkout", "-q", base)
        run_git(repo, "mv", src, moved)
        commit_all(repo, "base: 純粋リネーム")

        # **side を第 1 親にする**（base を第 1 親にすると、その木に src が無いので
        # is_metadata_only_change が比較できず免除が効かない）。
        run_git(repo, "checkout", "-q", "side")
        git_allow_fail(repo, "merge", "--no-commit", base)
        run_git(repo, "rm", "-q", "-f", "--ignore-unmatch", moved)
        write_src("2026-01-03")  # 解決も frontmatter だけ＝免除対象
        merge = commit_all(repo, "merge: 元の名前を残す")
        assert len(run_git(repo, "rev-parse", f"{merge}^@").split()) == 2, "前提: 2 親のマージ"

        listed = run_git(repo, "log", "--format=%H", "--", src).split()
        assert len(listed) >= 4, f"前提: リネームコミットまで列挙されること: {listed}"

        # **distill を c1 の手前に置き、「答えが c1」を正の assert で見る。**
        # `code == 0`（＝c1 は祖先なので STALE にならない）だけで見ると、
        # `continue` を `return None` に変える **fail-open 変異**（stale 判定が丸ごと
        # スキップされて warning に落ちる）と区別が付かない——実測で 184 ケース全通過した。
        write_doc(repo, "docs/knowledge/a.md", ["D19"], [FIRST_ADR, src], pre)
        write_registry(repo, pre)
        code, out = check(repo)
        assert code == 1, f"c1 が答えなら distill(pre) より後なので STALE になるはず:\n{out}"
        assert c1[:7] in out, (
            "答えが c1 になっていない（continue を return sha にしていないか。"
            f"その場合リネーム元としてしか現れないコミットに帰属する）:\n{out}"
        )
        assert "履歴が無く" not in out, (
            f"stale 判定が丸ごとスキップされている（continue を return None にしていないか）:\n{out}"
        )
    finally:
        shutil.rmtree(repo)


def test_rename_inside_merge_is_treated_as_content_change() -> None:
    """マージ内での純粋なリネームは R100 免除が効かず、偽の STALE になる（既知の限界）。

    combined diff はリネームを `RR` として出す（`R100` ではない）ので `scan_last_content_change`
    の免除分岐に当たらず、リネーム元も取れない。fail-closed 側なので実害は小さいが、
    ADR 0073 規模の移設をマージコミット内でやると大量に発火する。ADR 0084 で
    「塞がず記録する」と決めたので、**現状の挙動として** pin する
    （将来塞ぐなら、このテストを反転させるのが正しい入口）。
    """
    repo = new_repo()
    try:
        baseline(repo)
        renamed = "docs/original-docs/0003-renamed.md"
        base = run_git(repo, "rev-parse", "--abbrev-ref", "HEAD")
        run_git(repo, "checkout", "-q", "-b", "side")
        (repo / "unrelated.md").write_text("s\n", encoding="utf-8")
        commit_all(repo, "side unrelated")
        run_git(repo, "checkout", "-q", base)
        (repo / "other.md").write_text("m\n", encoding="utf-8")
        first_parent = commit_all(repo, "main unrelated")
        run_git(repo, "merge", "-q", "side", "--no-commit")
        run_git(repo, "mv", FIRST_ADR, renamed)  # 内容は変えない
        merge = commit_all(repo, "merge with rename")
        write_doc(repo, "docs/knowledge/a.md", ["D19"], [renamed], first_parent)
        # **レジストリの sources もリネーム後へ追従させる。** 既定のままだと消えたパスを
        # 指して「sources のパスが実在しない」が必ず出るので、`code == 1` が挙動によらず
        # 常に成立し、下の assert のメッセージが一度も表示されない（4 巡目レビューで実測）。
        write_registry(repo, first_parent, sources=[renamed])
        code, out = check(repo)
        regressed = (
            "マージ内リネームが STALE にならなかった。**`path_status` を第 1 親比較"
            "（`--first-parent` / `-m`）へ変えていないか。** その場合リネームが `R100` に見えて"
            "免除が効き **STALE が消える＝fail-open**（実測。片側だけの変更で偽 STALE は出ない）。"
            "既知の限界が直った合図ではないので、このテストを反転させてはいけない:\n"
            f"{out}"
        )
        assert code == 1, regressed
        assert "STALE" in out and merge[:7] in out, regressed
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
