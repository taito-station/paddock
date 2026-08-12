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

import re
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
) -> None:
    """レジストリを書く。`docs` は割当索引の行（既定は baseline の 1 本）。

    索引は checker が実ファイルと 1 対 1 で突き合わせるので、文書を足すテストは
    ここにも行を足す必要がある。
    """
    rows = docs if docs is not None else [("knowledge/a.md", ["D19"])]
    index = "\n".join(f"| {rel} | [{', '.join(classes)}] |" for rel, classes in rows)
    (repo / "docs/knowledge/doc-classes.md").write_text(
        REGISTRY_TEMPLATE.format(sha=sha, d08=d08, d19=d19, d22=d22, index=index),
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
        assert "履歴を辿れず" in out, out
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


def test_subdirectory_md_is_reported_as_unchecked() -> None:
    """サブディレクトリの `.md` は**完全に無検査**。3 つの warning 経路のうち最も危険なので固定する。

    glob が非再帰なので、`docs/knowledge/sub/x.md` は doc_class も sources も stale も
    一切検査されない。可視化の warning が消えると、文書がまるごと検査対象外になったことに
    誰も気づけなくなる。
    """
    repo = new_repo()
    try:
        baseline(repo)
        (repo / "docs/knowledge/sub").mkdir()
        (repo / "docs/knowledge/sub/x.md").write_text("# 検査対象外\n", encoding="utf-8")
        code, out = check(repo)
        assert code == 0, f"サブディレクトリ配置は error にしない: {out}"
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
        assert "履歴を辿れず" not in out, f"履歴を辿れなくなっている:\n{out}"
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
