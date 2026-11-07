#!/usr/bin/env python3
"""knowledge/ 配下の frontmatter と sources 追従（stale）を機械検査する。

`rules/hve/knowledge-maturity.md` が定義する frontmatter 標準（単一の
`distilled_from_sha` による祖先判定、Conflict の扱い）と `rules/hve/artifact-management.md`
の「機械検査」を実装する。導入先 paddock の `scripts/check-doc-classes.py` が持つ
汎用部分（frontmatter 解析・stale 判定・リネーム/メタデータ/ピン更新のみの変更の
スキップ）を移植し、paddock 固有の文書クラス登録簿・REQ-ID・相対リンク検査は含まない。

検査項目:
  1. `--required`（既定 title,status,kind,sources,distilled_from_sha,updated）の
     frontmatter 項目が欠落・空でないか。値が空リスト（裸の `key:`）・空の dict
     でも「空」として扱う（`sources` は専用の (4) に委ねて二重報告しない）     [error]
  2. `distilled_from_sha` が文書ごとに 1 つの sha（str）になっているか。旧標準の
     source ごとのマップ形式（`distilled_from_sha:` の下にインデントされた
     `path: sha` 行）や list 形式は error（ADR 0014）。str であっても
     `^[0-9a-f]{7,40}$`（小文字 16 進・7〜40 桁）に合わない値（`HEAD`・`main` 等の
     可変参照、大文字 16 進、短すぎる値）は error。ただし
     `--allow-empty-sources-with-decision-log` で空が許される文書は除く        [error]
  3. sources に列挙したパスがリポジトリ相対の正規形で、実在し、大文字小文字まで
     実ファイルと一致し、シンボリックリンクでないか（リポジトリの中を指している
     ものも含めて使えない。stale 判定（5）がリンク先の変更を追えないため）      [error]
  4. sources が空か（既定は error。`--allow-empty-sources-with-decision-log` を
     付けたときだけ、本文に `## 決定ログ` 見出し（行頭・コードフェンス外）を持つ
     文書は sources・distilled_from_sha が空・欠落でもよい）                  [error]
  5. stale: 各 source の最後の「内容変更」コミットが、文書の `distilled_from_sha`
     を解決した commit の祖先かどうか（`git merge-base --is-ancestor`）。
     `distilled_from_sha` の形式が不正なとき（(2) で error 済み）は、この判定自体を
     行わない（「形式不正」と「解決できない」を二重報告しない）。`rev-parse` で
     解決した結果のフル sha が書かれた値で始まらない場合（同名の tag・branch 等の
     ref に解決された）も error にする。`merge-base --is-ancestor` の終了コードが
     1（祖先でない＝STALE）でも 0（祖先）でもないときは、判定不能の別の error に
     する（STALE 行の書式で報告すると後続スクリプトが誤って解釈するため）       [error]

「内容変更ではない」として遡る（stale 判定の遡上をスキップする）のは次の 3 種類:
  - R100（内容差分ゼロのリネーム）
  - frontmatter のメタデータだけの変更（doc_class/tags/sources/distilled_from_sha/updated。
    status と kind の変更は下流に伝えるべき信号なので内容変更として扱う）
  - `.github/workflows/*.y(a)ml` の `uses: owner/repo@<40hex>` ピン留め SHA 更新だけの変更

status が `Conflict` の文書は、stale 判定の結果を error にせず「解消待ち」として
サマリーに別集計する（ADR 0011: Conflict の文書は sha を蒸留前に保つ運用のため、
放っておくと stale になり続ける。これは意図した状態であって違反ではない）。

**パース契約**: STALE の行は次の書式で固定する。`bump-distilled-sha.py` 等の
後続スクリプトがこの書式をパースするので、文言を変える場合は呼び出し側も直すこと。

  ✗ <rel>: STALE ← <src> が distilled_from_sha(<値>) より後に更新されている（<7桁>）。
  差分マージして sha/日付を更新する

使い方:
  check-knowledge.py [--dir knowledge] [--exclude PATTERN ...]
                      [--required a,b,c] [--allow-empty-sources-with-decision-log]
                      [--warn-only] [--max-pages N] [--max-renames N]

  `--dir` を省略すると既定で `knowledge/` を検査する。既定の `knowledge/` が
  存在しない場合は対象 0 本として exit 0 にする（`--dir` を明示指定して存在
  しない場合は exit 2）。

終了コード:
  0: 違反なし（`--dir` を省略し既定の knowledge/ が存在しない場合も対象 0 本として 0）
  1: 違反あり（--warn-only を付けると 0 に落ちる）
  2: 判定不能（shallow clone で履歴が足りない・git リポジトリ外・`--dir` を明示
     指定したディレクトリが存在しない・引数不正など。--warn-only でも 0 には
     ならない）
"""

from __future__ import annotations

import argparse
import functools
import re
import subprocess
import sys
from pathlib import Path

# --- git ヘルパー -----------------------------------------------------------
# 必ずリポジトリルートで実行する（cwd 依存だと pathspec が cwd 相対に解決され、
# サブディレクトリから呼んだときに stale 判定が無言でスキップされる）。
_ROOT: Path = Path(".")


def git(*args: str) -> "subprocess.CompletedProcess[str]":
    # core.quotePath=false を常に付ける。既定だと非 ASCII パスが "\346\234\200..." の
    # クォート表記になり、終点一致を使う path_status 等が外れる。
    return subprocess.run(
        ["git", "-c", "core.quotePath=false", *args],
        cwd=_ROOT,
        capture_output=True,
        text=True,
    )


def git_raw(*args: str) -> "subprocess.CompletedProcess[bytes]":
    """git の出力を**バイト列**で取る（理由は decode_preserving の docstring）。"""
    return subprocess.run(
        ["git", "-c", "core.quotePath=false", *args], cwd=_ROOT, capture_output=True
    )


def repo_root() -> Path:
    proc = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True
    )
    if proc.returncode != 0:
        print("git リポジトリ外では実行できない", file=sys.stderr)
        sys.exit(2)
    return Path(proc.stdout.strip())


def is_shallow() -> bool:
    return git("rev-parse", "--is-shallow-repository").stdout.strip() == "true"


# --- frontmatter 解析 --------------------------------------------------------
# 外部ライブラリ（PyYAML）を使わない。frontmatter は限定的な構造しか取らないため
# 正規表現で読める。キー名は固定しない（--required が任意のキーを指せるため）。
RE_LIST_HEAD = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*):\s*$")
RE_LIST_ITEM = re.compile(r"^\s+-\s+(\S+)")
RE_FLOW_LIST = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*):\s*\[([^\]]*)\]\s*(?:#.*)?$")
RE_SCALAR = re.compile(r'^([A-Za-z_][A-Za-z0-9_]*):\s*"?([^"#]*?)"?\s*(?:#.*)?$')

# 旧標準の source ごとのマップ形式（`distilled_from_sha:` の下にインデントされた
# `path: sha` 行。ADR 0014 で廃止）を検出する。ブロックリストの `- item` 行は
# 除外する（先頭が `-` の行は list 形式として別途 isinstance(..., list) で検出）。
RE_OLD_MAP_ENTRY = re.compile(r"^\s+[^\s:#-][^:]*:\s*\S")
# distilled_from_sha の値が取り得る形式。小文字 16 進・7〜40 桁（HEAD/main 等の
# 可変参照や大文字 16 進・短すぎる値を拒むことで stale 判定の無効化を防ぐ）。
RE_SHA = re.compile(r"^[0-9a-f]{7,40}$")


def parse_frontmatter(text: str) -> dict:
    """frontmatter を dict で返す。無ければ空 dict。

    値がリストのキー（ブロックの `- item` 形式、またはフローの `[a, b]` 形式）は
    list、それ以外は str になる。
    """
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        return {}
    out: dict = {}
    list_key: "str | None" = None
    for line in lines[1:]:
        if line.strip() == "---":
            break
        if line.lstrip().startswith("#"):
            continue
        if list_key is not None:
            item = RE_LIST_ITEM.match(line)
            if item:
                out[list_key].append(item.group(1))
                continue
            list_key = None
        head = RE_LIST_HEAD.match(line)
        if head:
            list_key = head.group(1)
            out[list_key] = []
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


def split_frontmatter(text: str) -> "tuple[str | None, str]":
    """(frontmatter 本体, それ以降の本文) を返す。frontmatter が無ければ (None, 全文)。"""
    lines = text.splitlines(keepends=True)
    if not lines or lines[0].strip() != "---":
        return None, text
    for i in range(1, len(lines)):
        if lines[i].strip() == "---":
            return "".join(lines[1:i]), "".join(lines[i + 1 :])
    return None, text


def frontmatter_blocks(fm: str) -> "dict[str, str]":
    """frontmatter を「キー → そのキーに属する行のかたまり」に分解する。"""
    out: dict[str, str] = {}
    key = None
    for line in fm.splitlines():
        head = re.match(r"^([A-Za-z_][A-Za-z0-9_]*):", line)
        if head:
            key = head.group(1)
            out[key] = line + "\n"
        elif key is not None:
            out[key] += line + "\n"
    return out


# 変わっても「その文書の内容が変わった」とは見なさないキー。status / kind は
# 意味を持つ変更（例: Confirmed → Conflict）なので対象外。
METADATA_KEYS = {"doc_class", "tags", "sources", "distilled_from_sha", "updated"}


@functools.lru_cache(maxsize=32)
def blob_at(sha: str, path: str) -> "bytes | None":
    """コミット sha 時点の path の中身（**バイト列**）。取れなければ None。

    バイト列で持つのは、CRLF⇄LF の変換や不正 UTF-8 を「差分なし」に潰さないため
    （`text=True` の universal newlines・`errors="replace"` はどちらも差分を潰す）。
    """
    proc = git_raw("show", f"{sha}:{path}")
    return proc.stdout if proc.returncode == 0 else None


def decode_preserving(raw: bytes) -> str:
    """git から取ったバイト列を**往復可能な形で**復号する（surrogateescape）。

    `errors="replace"` だと異なる不正バイトが同じ U+FFFD に潰れ、別内容が
    「一致」に見えてしまう（frontmatter の本文比較・`uses:` 行の比較で使う）。
    """
    return raw.decode("utf-8", "surrogateescape")


def is_metadata_only_change(sha: str, path: str) -> bool:
    """そのコミットの変更が frontmatter のメタデータだけかを判定する。"""
    new_raw, old_raw = blob_at(sha, path), blob_at(f"{sha}^", path)
    if new_raw is None or old_raw is None:
        return False  # 初回追加や親を辿れない場合は内容変更として扱う
    new_text, old_text = decode_preserving(new_raw), decode_preserving(old_raw)
    new_fm, new_body = split_frontmatter(new_text)
    old_fm, old_body = split_frontmatter(old_text)
    if new_fm is None or old_fm is None or new_body != old_body:
        return False
    new_blocks, old_blocks = frontmatter_blocks(new_fm), frontmatter_blocks(old_fm)
    changed = {
        k for k in set(new_blocks) | set(old_blocks) if new_blocks.get(k) != old_blocks.get(k)
    }
    return bool(changed) and changed <= METADATA_KEYS


# 例外: GitHub Actions の `uses:` ピン留め SHA 更新だけの変更（paddock の移行に必要）。
RE_WORKFLOW_PATH = re.compile(r"^\.github/workflows/[^/]+\.ya?ml$")
RE_USES_PIN = re.compile(
    r"^(\s*(?:-\s+)?uses:\s+)([^@\s/]+/[^@\s/]+)@([0-9a-fA-F]{40})([ \t]+#[ \t]*v?[0-9][0-9A-Za-z._+-]*)?$"
)


def is_pin_only_change(sha: str, path: str) -> bool:
    """そのコミットの変更が `uses:` のピン留め SHA 更新だけかを判定する。"""
    if not RE_WORKFLOW_PATH.match(path):
        return False
    new_raw, old_raw = blob_at(sha, path), blob_at(f"{sha}^", path)
    if new_raw is None or old_raw is None:
        return False
    new_lines, old_lines = new_raw.split(b"\n"), old_raw.split(b"\n")
    if len(new_lines) != len(old_lines):
        return False  # 行の増減はジョブ構成の変更
    hex_changed = False
    for new_line, old_line in zip(new_lines, old_lines):
        if new_line == old_line:
            continue
        new_pin = RE_USES_PIN.match(decode_preserving(new_line))
        old_pin = RE_USES_PIN.match(decode_preserving(old_line))
        if not new_pin or not old_pin:
            return False
        if new_pin.group(1, 2) != old_pin.group(1, 2):
            return False
        if new_pin.group(3) != old_pin.group(3):
            hex_changed = True
    return hex_changed


class GitFailed(Exception):
    """git コマンドそのものが失敗した（走査を continue して握り潰すと fail-open になる）。"""


def path_status(sha: str, path: str) -> "tuple[str | None, str | None]":
    """コミット sha における path の (status, リネーム元) を返す。

    マージに対する `git show` の既定（combined diff）に依存する。詳細な理由は
    paddock の `scripts/check-doc-classes.py` の同名関数の docstring を参照。

    `-z` で読み NUL 区切りでフィールドを分解する。`-z` なしだとファイル名に含まれる
    二重引用符やバックスラッシュが C クォートされ（`core.quotePath=false` は非
    ASCII だけを対象にするので効かない）、`path` との終点一致が外れて stale 判定が
    無言で warning に落ちる。
    """
    proc = git_raw("show", "--format=", "--name-status", "-M100%", "-z", sha)
    if proc.returncode != 0:
        raise GitFailed(
            f"git show --name-status が失敗した（{decode_preserving(proc.stderr).strip()[:200]}）"
        )
    fields = [decode_preserving(b) for b in proc.stdout.split(b"\0") if b]
    i = 0
    while i < len(fields):
        status = fields[i]
        if status.startswith("R"):
            if i + 2 >= len(fields):
                break
            old, new = fields[i + 1], fields[i + 2]
            if new == path:
                return status, old
            i += 3
            continue
        if i + 1 >= len(fields):
            break
        current = fields[i + 1]
        if current == path:
            return status, None
        i += 2
    return None, None


class ScanAborted:
    """`scan_last_content_change` が走査を完遂できなかったことを表す番兵。

    SHA 文字列とは型で区別する。`is None` だけで判定すると番兵が SHA として
    `merge-base --is-ancestor` に渡り、偽の STALE を出す事故につながる。
    """

    __slots__ = ("reason",)

    def __init__(self, reason: str) -> None:
        self.reason = reason

    def __repr__(self) -> str:
        return f"<scan-aborted: {self.reason}>"


@functools.lru_cache(maxsize=None)
def last_content_change(
    path: str, limit: int = 40, max_renames: int = 10, max_pages: int = 25
) -> "str | ScanAborted | None":
    """path の内容が最後に変わったコミットの SHA（git の失敗は ScanAborted に寄せる）。"""
    try:
        return scan_last_content_change(path, limit, max_renames, max_pages)
    except GitFailed as exc:
        return ScanAborted(str(exc))


def scan_last_content_change(
    path: str, limit: int, max_renames: int, max_pages: int
) -> "str | ScanAborted | None":
    """path の**内容**が最後に変わったコミットの SHA。

    戻り値は 3 通り。「走査を完遂できなかった」を「履歴が無い」に混ぜないのが要点
    （混ぜると fail-open になる）。

      - SHA         : 内容が最後に変わったコミット
      - None        : 履歴が無い（未コミット・履歴の尽き・shallow）→ 呼び出し側は warning
      - ScanAborted : 走査を完遂できなかった（ページ予算・リネーム予算・git の失敗）
                      → 呼び出し側は error（fail-closed）
    """
    current = path
    tip = "HEAD"
    skip = 0
    renames = 0
    pages = 0
    while True:
        if pages >= max_pages:
            return ScanAborted(
                f"除外対象が続きすぎてページ予算（max_pages={max_pages}）を使い切った"
            )
        pages += 1
        proc = git(
            "log", f"--max-count={limit}", f"--skip={skip}", "--format=%H", tip, "--", current
        )
        if proc.returncode != 0:
            return ScanAborted(f"git log が失敗した（{proc.stderr.strip()[:200]}）")
        shas = proc.stdout.split()
        if not shas:
            return None  # 履歴が尽きた
        renamed = False
        for sha in shas:
            status, rename_src = path_status(sha, current)
            if status is None:
                # リネーム元としてしか現れないコミット、または非正規形パス。
                # `return sha` に変えると偽の STALE が出る（load-bearing な continue）。
                continue
            if status == "R100":
                if not rename_src:
                    return sha  # リネーム元を取れない＝判定できないので内容変更扱い
                current = rename_src
                tip = f"{sha}^"
                skip = 0  # パスが変わったので窓の位置は取り直す
                renames += 1
                renamed = True
                break
            if is_pin_only_change(sha, current):
                continue
            if is_metadata_only_change(sha, current):
                continue
            return sha
        if renamed:
            if renames >= max_renames:
                return ScanAborted(
                    f"リネームを辿りすぎてリネーム予算（max_renames={max_renames}）を使い切った"
                )
            continue
        if len(shas) < limit:
            return None  # この系列は全部「内容変更ではない」で、履歴も尽きた
        skip += limit  # 窓を使い切っただけ。まだ先に履歴があるので次のページへ


def case_exact(path: Path, root: Path) -> bool:
    """root から path までの**全成分**が実在の名前と大文字小文字まで一致するかを見る。

    macOS(APFS) は大文字小文字を区別しないので `exists()` が通り、Linux だけが落ちる。
    """
    try:
        rel_parts = path.relative_to(root).parts
    except ValueError:
        return True  # リポジトリ外は別の error で扱う
    current = root
    for part in rel_parts:
        try:
            if part not in {entry.name for entry in current.iterdir()}:
                return False
        except OSError:
            return True  # 読めないディレクトリは判定しない
        current = current / part
    return True


def repo_relative_path_error(raw: str) -> "str | None":
    """`sources` に書けるパスでない理由を返す（正常なら None）。"""
    if raw.startswith("/") or ".." in Path(raw).parts:
        return "リポジトリ相対パスで書く（絶対パス・`..` は使わない）"
    if Path(raw).as_posix() != raw:
        return "正規形で書く（`./` や重複スラッシュを使わない）"
    return None


# --- 決定ログ見出しの検出 -----------------------------------------------------
RE_FENCE = re.compile(r"^(`{3,}|~{3,})")
RE_DECISION_LOG_HEADING = re.compile(r"^##[ \t]+決定ログ[ \t]*$")


def has_decision_log_heading(body: str) -> bool:
    """本文（frontmatter を除く）が `## 決定ログ` 見出しを行頭・コードフェンス外に持つか。"""
    fence: "tuple[str, int] | None" = None
    for line in body.splitlines():
        opener = RE_FENCE.match(line.strip())
        if opener:
            token = opener.group(1)
            if fence is None:
                fence = (token[0], len(token))
            elif token[0] == fence[0] and len(token) >= fence[1]:
                fence = None
            continue
        if fence is None and RE_DECISION_LOG_HEADING.match(line):
            return True
    return False


# --- 走査対象の絞り込み -------------------------------------------------------
DEFAULT_REQUIRED = "title,status,kind,sources,distilled_from_sha,updated"
DEFAULT_EXCLUDE = ["adr/", "README.md"]


def is_excluded(rel_to_dir: str, patterns: "list[str]") -> bool:
    """`rel_to_dir`（--dir からの相対パス）が除外パターンに当たるか。

    末尾が `/` のパターンはディレクトリ接頭辞、それ以外は完全一致で比べる。
    """
    for pat in patterns:
        if pat.endswith("/"):
            if rel_to_dir.startswith(pat):
                return True
        elif rel_to_dir == pat:
            return True
    return False


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "knowledge/ 配下の frontmatter 必須項目・sources 実在・"
            "祖先判定による stale を検査する。"
        )
    )
    parser.add_argument(
        "--dir", default=None,
        help="検査対象ディレクトリ（リポジトリルート相対。既定: knowledge。再帰的に走査する。"
             "既定の knowledge/ が存在しない場合は対象 0 本として exit 0 にする。"
             "明示指定したディレクトリが存在しない場合は exit 2 にする）",
    )
    parser.add_argument(
        "--exclude", action="append", default=None,
        help="除外パターン（--dir からの相対。末尾 `/` でディレクトリ接頭辞。"
             "複数指定可。指定すると既定（adr/ と README.md）を置き換える。"
             "既定を残すときは併せて指定する）",
    )
    parser.add_argument(
        "--required", default=DEFAULT_REQUIRED,
        help=f"必須 frontmatter 項目（カンマ区切り。既定: {DEFAULT_REQUIRED}）",
    )
    parser.add_argument(
        "--allow-empty-sources-with-decision-log", action="store_true",
        help="`## 決定ログ` 見出しを持つ文書は sources・distilled_from_sha が"
             "空・欠落でもよい（stale 判定を行わない）",
    )
    parser.add_argument(
        "--warn-only", action="store_true",
        help="違反（error）があっても exit 0 にする（判定不能＝exit 2 は対象外）",
    )
    parser.add_argument(
        "--max-pages", type=int, default=25,
        help="stale 走査のページ予算（既定 25。超過は ScanAborted = error）",
    )
    parser.add_argument(
        "--max-renames", type=int, default=10,
        help="stale 走査のリネーム予算（既定 10。超過は ScanAborted = error）",
    )
    return parser


def main(argv: "list[str] | None" = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    global _ROOT
    _ROOT = repo_root()
    root = _ROOT

    exclude_patterns = args.exclude if args.exclude is not None else list(DEFAULT_EXCLUDE)
    required_keys = [k.strip() for k in args.required.split(",") if k.strip()]

    dir_explicit = args.dir is not None
    target_dir_rel = args.dir if dir_explicit else "knowledge"
    target_dir = root / target_dir_rel

    default_dir_missing = False
    if not target_dir.is_dir():
        if dir_explicit:
            print(f"検査対象ディレクトリが見つからない: {target_dir_rel}", file=sys.stderr)
            return 2
        default_dir_missing = True  # --dir 省略時は対象 0 本として続行する（exit 0）

    targets: list[Path] = []
    if not default_dir_missing:
        for p in sorted(target_dir.rglob("*.md")):
            if not p.is_file():
                continue
            rel_to_dir = p.relative_to(target_dir).as_posix()
            if is_excluded(rel_to_dir, exclude_patterns):
                continue
            targets.append(p)

    shallow = is_shallow()
    errors: list[str] = []
    warnings: list[str] = []
    pending_conflicts: list[str] = []
    shallow_blocked = False

    for path in targets:
        rel = path.relative_to(root).as_posix()
        text = path.read_text(encoding="utf-8")
        fm = parse_frontmatter(text)
        if not fm:
            errors.append(f"{rel}: frontmatter が無い")
            continue

        fm_raw, body = split_frontmatter(text)
        has_decision_log = has_decision_log_heading(body)
        allow_empty = args.allow_empty_sources_with_decision_log and has_decision_log

        # distilled_from_sha の形式判定。旧マップ形式は parse_frontmatter 上では
        # 空リストに潰れてしまうので、raw frontmatter のインデント行を別途見る。
        distilled_value = fm.get("distilled_from_sha")
        distilled_raw_block = frontmatter_blocks(fm_raw or "").get("distilled_from_sha", "")
        distilled_is_old_map = any(
            RE_OLD_MAP_ENTRY.match(line) for line in distilled_raw_block.splitlines()[1:]
        )
        distilled_malformed = distilled_is_old_map or (
            isinstance(distilled_value, list) and len(distilled_value) > 0
        )
        # 形式が不正（旧マップ/list 形式、または sha 以外の値）全般。(5) の stale
        # 判定はこれが真なら行わない（「形式不正」と「解決できない」を二重報告しない）。
        distilled_format_invalid = distilled_malformed or (
            isinstance(distilled_value, str)
            and distilled_value.strip() != ""
            and not RE_SHA.match(distilled_value.strip())
        )

        # (1) 必須項目
        for key in required_keys:
            if key in ("sources", "distilled_from_sha") and allow_empty:
                continue
            if key not in fm:
                errors.append(f"{rel}: 必須項目 {key} が無い")
                continue
            if key == "sources":
                continue  # 空判定は (4) の専用チェックに委ねる（二重報告しない）
            if key == "distilled_from_sha" and distilled_malformed:
                continue  # 形式エラーとして別途報告する（二重報告しない）
            value = fm[key]
            if isinstance(value, str):
                if value.strip() == "":
                    errors.append(f"{rel}: 必須項目 {key} が空")
            elif isinstance(value, (list, dict)) and not value:
                errors.append(f"{rel}: 必須項目 {key} が空")

        # (2) distilled_from_sha の形式（旧マップ形式・list 形式、または sha 以外の
        # 値。allow_empty で空が免除される文書は、値が実際に空ならここで何もしない）
        if distilled_malformed:
            errors.append(
                f"{rel}: distilled_from_sha の形式が不正"
                "（文書ごとに 1 つの sha で書く。ADR 0014）"
            )
        elif isinstance(distilled_value, str) and distilled_value.strip():
            if not RE_SHA.match(distilled_value.strip()):
                errors.append(
                    f"{rel}: distilled_from_sha が sha 形式でない"
                    "（7〜40 桁の小文字 16 進で書く。HEAD/main 等の可変参照は使えない）"
                    f" → {distilled_value.strip()}"
                )

        # (4) 空 sources（key が無い場合は上の必須項目チェックに委ねて二重報告しない）
        sources: "list[str]" = fm.get("sources", [])
        if "sources" in fm and not sources and not allow_empty:
            errors.append(f"{rel}: sources が空（由来を辿れない）")

        # (3) sources の実在・正規形・大文字小文字・symlink（空・欠落のどちらでも、
        # 列挙された要素だけは常に検査する）。symlink はリポジトリの中を指して
        # いても使えない（stale 判定の履歴走査がリンク先の変更を追えないため）。
        for src in sources:
            path_error = repo_relative_path_error(src)
            if path_error:
                errors.append(f"{rel}: sources は{path_error} → {src}")
            elif (root / src).is_symlink():
                errors.append(
                    f"{rel}: sources にシンボリックリンクは使えない。実体のパスを書く → {src}"
                )
            elif not (root / src).is_file():
                errors.append(f"{rel}: sources のパスが実在しない → {src}")
            elif not case_exact((root / src).resolve(), root.resolve()):
                errors.append(f"{rel}: sources の大文字小文字が実ファイルと違う → {src}")

        # 免除は sources と distilled_from_sha が揃って空のときだけ。sources を
        # 持つのに sha が無いと stale 判定から黙って外れるので error にする
        # （免除でないときは上の必須項目チェックが報告済み。形式が不正なときは
        # 上の (2) が報告済みなので二重報告しない）。
        if allow_empty and sources and not distilled_malformed and not distilled_value:
            errors.append(f"{rel}: sources があるのに distilled_from_sha が無い")

        status = fm.get("status", "")
        if status == "Conflict":
            # ADR 0011: Conflict の文書は sha を蒸留前に保つ運用で、意図的に
            # stale のまま留まる。違反ではなく「解消待ち」として別集計する。
            pending_conflicts.append(rel)
            continue

        # (5) stale。sources・distilled_from_sha のどちらかが無ければ判定しない
        # （allow_empty による免除時はこれが意図した状態。免除でない欠落・形式が
        # 不正な値は上のチェックで既に error 済みなので、ここで重複報告しない）。
        if distilled_format_invalid or not isinstance(distilled_value, str):
            continue
        distilled = distilled_value
        if not distilled or not sources:
            continue

        resolved = git("rev-parse", "--verify", "--quiet", f"{distilled}^{{commit}}")
        if resolved.returncode != 0:
            if shallow:
                errors.append(
                    f"{rel}: distilled_from_sha '{distilled}' を解決できない"
                    "（shallow clone のため判定不能。fetch-depth: 0 で取得し直す）"
                )
                shallow_blocked = True
            else:
                errors.append(f"{rel}: distilled_from_sha '{distilled}' を解決できない")
            continue
        distilled_full = resolved.stdout.strip()
        if not distilled_full.startswith(distilled):
            # rev-parse は ref 名も受け付けるので、書いた値と同名の tag・branch が
            # あると無関係な commit に解決される（本物の sha ならフル sha が必ず
            # 書いた値で始まる）。
            errors.append(
                f"{rel}: distilled_from_sha が同名の ref（tag・branch）に解決された"
                f" → {distilled}"
            )
            continue

        for src in sources:
            if not (root / src).is_file():
                continue  # 実在チェックで既に error 済み
            changed = last_content_change(
                src, max_renames=args.max_renames, max_pages=args.max_pages
            )
            if isinstance(changed, ScanAborted):
                # 走査側の都合（fail-closed）。warning に落とすと「除外対象の
                # コミットを積めば検査が消える」経路が一段外側で再現する。
                errors.append(
                    f"{rel}: {src} の履歴走査を完遂できず stale 判定が行われていない"
                    f"（{changed.reason}）"
                )
                continue
            if changed is None:
                if shallow:
                    errors.append(
                        f"{rel}: {src} の履歴を辿れない（shallow clone のため判定不能。"
                        "fetch-depth: 0 で取得し直す）"
                    )
                    shallow_blocked = True
                else:
                    warnings.append(
                        f"{rel}: {src} の履歴が無く stale 判定を実施できなかった"
                        "（未コミット / 履歴の尽き）"
                    )
                continue
            mb = git("merge-base", "--is-ancestor", changed, distilled_full)
            if mb.returncode == 1:
                errors.append(
                    f"{rel}: STALE ← {src} が distilled_from_sha({distilled}) より後に更新されている"
                    f"（{changed[:7]}）。差分マージして sha/日付を更新する"
                )
            elif mb.returncode != 0:
                # 0（祖先）・1（祖先でない）以外は祖先判定そのものが失敗している。
                # STALE 行の書式で報告すると bump-distilled-sha.py 等が誤ってパース
                # するので、別の書式の error にする（判定不能）。
                errors.append(
                    f"{rel}: {src} の祖先判定ができない"
                    f"（git merge-base が異常終了した: {mb.stderr.strip()[:200]}）"
                )

    # --- 報告 ---
    for w in warnings:
        print(f"警告: {w}", file=sys.stderr)
    # 解消待ちの文書を列挙する（AKM の報告に載せるため）。`✗ ` で始めない
    # （bump-distilled-sha.py は `✗ ` 行を違反としてパースする）。
    for rel in pending_conflicts:
        print(f"解消待ち: {rel}（status: Conflict）", file=sys.stderr)

    if shallow_blocked:
        print("", file=sys.stderr)
        for e in errors:
            print(f"✗ {e}", file=sys.stderr)
        print("", file=sys.stderr)
        print(
            "✗ 判定不能（shallow clone のため stale 判定を完遂できない文書がある。"
            f"fetch-depth: 0 で取得し直す）（警告 {len(warnings)} 件 / "
            f"解消待ち {len(pending_conflicts)} 件）",
            file=sys.stderr,
        )
        return 2

    if errors:
        print("", file=sys.stderr)
        for e in errors:
            print(f"✗ {e}", file=sys.stderr)
        print("", file=sys.stderr)
        print(
            f"✗ {len(errors)} 件の不整合（警告 {len(warnings)} 件 / "
            f"解消待ち {len(pending_conflicts)} 件）",
            file=sys.stderr,
        )
        if args.warn_only:
            print("  --warn-only のため 0 で終了する", file=sys.stderr)
            return 0
        return 1

    print(
        f"✓ knowledge 文書の整合を確認"
        f"（{len(targets)} 本 / 警告 {len(warnings)} 件 / 解消待ち {len(pending_conflicts)} 件）"
    )
    if default_dir_missing:
        print("knowledge/ が無いので対象なし")
    return 0


if __name__ == "__main__":
    sys.exit(main())
