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
  6. stale: sources の最終「内容変更」が distilled_from_sha に含まれているか  [error]
  7. active なのに文書 0 本のクラス（充足ギャップ）                           [warning]
  8. REQ 表（要件 ID）の書式・一意性・status・Confirmed の検証手段            [error]
  9. 本文中の相対リンクが実在するか（REQ 表の外も見る）                       [error]
 10. doc-classes.md の割当索引と実ファイルの doc_class の突合                 [error]
 11. REQ 表の出典が名指しした一次資料が frontmatter の sources にも有るか     [error]
 12. ADR がどこかの sources から参照されているか（orphan ADR）               [error]

11・12 は stale 検査（6）の網羅性の穴を両側から塞ぐ（ADR 0083）。stale は sources に
**挙がっている**行しか見ないので、`sources` から行を消せば stale も消えるし、ADR を足したときに
載せ忘れても何も起きない。11 は「本文で根拠に挙げた ADR」、12 は「ADR 全体」を watch 対象に
入れることを保証する。12 の例外は doc-classes.md の宣言表（`adr-orphan-exceptions`）で持つ——
スクリプト内の定数にすると、例外を増やす行為が文書レビューに乗らない。

6 は「内容が変わっていないコミット」を比較対象から除外する（規約は docs/knowledge/README.md
の例外 1 / 1b / 1d）。rename-only（内容差分ゼロ）を除外しないと ADR 0073 の ADR 移動だけで
20 本が一斉に stale 判定になる。git log --follow では吸収できない——--follow はリネームより
前へ履歴を遡らせるだけで、「最終コミット」がリネームコミットになる事実は変わらない。
frontmatter のメタデータだけの変更（例外 1b）と、`uses:` のピン留め SHA 更新だけの変更
（例外 1d）も同様に遡る。後者が無いと dependabot の Actions 更新 PR が構造的に永久に赤になる。

依存は標準ライブラリのみ（PyYAML を使わない）。CI の predict-check ジョブと同じ前提で、
frontmatter は限定的な構造しか取らないため正規表現で足りる。

使い方:
  scripts/check-doc-classes.py               # 検査（error があれば非ゼロ終了）
  scripts/check-doc-classes.py check         # 同上
  scripts/check-doc-classes.py --warn-only   # error も警告として報告し常に 0 で終了
                                             # （例外: 検査が成立しない 2 つ——マーカー欠落と
                                             #   ADR 0 件——は fail-closed で 1）
"""

import bisect
import functools
import re
import subprocess
import sys
from collections.abc import Callable, Iterator
from pathlib import Path

USAGE = """check-doc-classes.py - 文書クラスと sources 追従の機械検査（ADR 0073）

使い方:
  scripts/check-doc-classes.py               # 検査（error があれば非ゼロ終了）
  scripts/check-doc-classes.py check         # 同上
  scripts/check-doc-classes.py --warn-only   # error も警告扱いにして常に 0 で終了
                                             # （例外: 検査そのものが成立しない 2 つは 1 で
                                             #  落とす——doc-classes.md のマーカー欠落
                                             #  〈表の範囲を切り出せない〉と ADR 0 件
                                             #  〈orphan 検査が丸ごと効いていない〉）

オプション:
  -h, --help   このヘルプ
"""

# クラス定義の正本。この 1 ファイルだけが「どのクラスが存在するか」を決める。
REGISTRY = Path("docs/knowledge/doc-classes.md")

# 検査対象のディレクトリ。両方とも「その場で knowledge」として frontmatter を持つ。
TARGET_DIRS = ("docs/knowledge", "docs/specifications")

# 一次資料層。`sources` が watch すべき蒸留元はここに限る（ADR 0083）。
ORIGINAL_DOCS = "docs/original-docs"
# ADR のファイル名は **0 埋め 4 桁**。issue 由来の一次資料（`382-...`）は 0 埋めしない規約
# （CLAUDE.md「命名は 2 系統」）。
#
# **`scripts/check-adr-numbers.sh` の `^0[0-9]{3}` と同一の述語**。意図して同じ形にしてある——
# 先頭 0 を落として `^\d{4}` にすると、issue 番号が 4 桁に届いた時点で `1024-foo.md` のような
# 一次資料が ADR と誤判定され、「例外表に登録しろ」という誤った助言つきで CI が落ちる
# （現在 issue は #613）。judge が 2 本に割れているのが、この PR 自身が塞いでいる second source。
# 末尾に `-` を要求しないのも向こうに合わせている（`00401-...` のような 0 埋めを誤った ADR も
# 母集合に入れる。採番の是正は check-adr-numbers.sh の担当で、こちらは被参照だけを見る）。
RE_ADR_FILENAME = re.compile(r"^0\d{3}")

# orphan 例外表の列。見出し行を完全一致で要求する（列名を変えるとデータ行として読まれる）。
ORPHAN_EXCEPTION_COLUMNS = ("ADR", "例外の理由")

# 走査から外すファイル。
#   README.md    : 規約そのもの。frontmatter のテンプレート例（0NNN-....md 等の
#                  存在しないパス）を含むため、frontmatter 系の検査は必ず偽陽性になる。
#                  ただし**本文リンクだけは検査する**（見本はコードフェンス内なので
#                  除外済み。規約の正本を唯一の無検査域にしない・#604）。
#   doc-classes.md: クラス定義そのもの。doc_class は持たない（sources/stale だけ検査する）。
EXCLUDED_FROM_DOC_CLASS = {"README.md", "doc-classes.md"}
# frontmatter 系の検査からは外すが、**本文リンクだけは必ず見る**（除外した文書が
# 唯一の無検査域にならないように、この集合をそのままリンク検査へ回す）。
EXCLUDED_ENTIRELY = {"README.md"}
# TARGET_DIRS の外だが**リンクだけは見る**ファイル。CLAUDE.md は毎セッション読まれる運用指示で、
# 用語集や仕様書への相対リンクを多数持つのに、ディレクトリ基準の走査からは外れていた。
EXTRA_LINK_TARGETS = ("CLAUDE.md",)

# frontmatter は限定的な構造しか取らないので正規表現で読む。書式は doc-classes.md が規定。
# 行末のインラインコメントを許す（規約が示すテンプレ自身がコメント付きで書かれているため、
# 許さないと「正本のテンプレをコピペすると checker が落ちる」という破綻が起きる）。
RE_FLOW_LIST = re.compile(r"^(doc_class|tags):\s*\[([^\]]*)\]\s*(?:#.*)?$")
RE_SOURCES_HEAD = re.compile(r"^sources:\s*$")
RE_SOURCES_ITEM = re.compile(r"^\s+-\s+(\S+)")
RE_SCALAR = re.compile(r'^(status|kind|distilled_from_sha|updated):\s*"?([^"#]*?)"?\s*(?:#.*)?$')
# レジストリの表。行数や見出し文言に依存しないようマーカーで範囲を切り出す。
RE_CLASS_ROW = re.compile(r"^\|\s*(D\d{2})\s*\|([^|]*)\|\s*(active|n/a)\s*\|\s*(\d+)\s*\|")
RE_NA_ROW = re.compile(r"^\|\s*(D\d{2})\s*\|")


# git コマンドの実行ディレクトリ。repo_root() で確定させる。
# **必ずリポジトリルートで実行する**。cwd 依存にすると `git log -- docs/...` の pathspec が
# cwd 相対に解決され、ルート以外から呼んだときに stale 判定が全件無言でスキップされて
# 「✓ 整合を確認」と表示したまま exit 0 する（fail-open）。
_ROOT: Path = Path(".")


def git(*args: str) -> "subprocess.CompletedProcess[str]":
    return subprocess.run(["git", *args], cwd=_ROOT, capture_output=True, text=True)


def git_raw(*args: str) -> "subprocess.CompletedProcess[bytes]":
    """git の出力を**バイト列**で取る（理由は blob_at の docstring）。"""
    return subprocess.run(["git", *args], cwd=_ROOT, capture_output=True)


def repo_root() -> Path:
    proc = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True
    )
    if proc.returncode != 0:
        sys.exit("git リポジトリ外では実行できない")
    return Path(proc.stdout.strip())


def is_shallow() -> bool:
    return git("rev-parse", "--is-shallow-repository").stdout.strip() == "true"


def extract_block(text: str, name: str) -> list[str]:
    """`<!-- name:begin -->` … `<!-- name:end -->` に挟まれた行を返す。"""
    begin, end = f"<!-- {name}:begin -->", f"<!-- {name}:end -->"
    if begin not in text or end not in text:
        sys.exit(f"{REGISTRY} に {begin} / {end} のマーカーが無い（表の範囲を切り出せない）")
    return text.split(begin, 1)[1].split(end, 1)[0].splitlines()


def parse_frontmatter(text: str) -> dict:
    """frontmatter を dict で返す。無ければ空 dict。

    `---` 直後の `#` 始まり行（specifications が持つ YAML コメント）は読み飛ばす。
    """
    lines = text.splitlines()
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


# 変わっても「その文書の内容が変わった」とは見なさないキー。すべてトレーサビリティ用の
# メタデータで、下流の knowledge が読み直すべき理由にならない。status / kind の変更は
# 意味を持つので、ここには入れない（例: Confirmed → Conflict は下流に伝えるべき信号）。
METADATA_KEYS = {"doc_class", "tags", "sources", "distilled_from_sha", "updated"}


# 「内容変更ではない」を判定する述語が同じ (sha, path) の前後ブロブを見るので共有する。
@functools.lru_cache(maxsize=32)
def blob_at(sha: str, path: str) -> "bytes | None":
    """コミット sha 時点の path の中身（**バイト列**）。取れなければ None。

    バイト列で持つ理由は 2 つあり、どちらも「差分を潰さない」ため。
      - `text=True` の universal newlines は `\\r\\n` を `\\n` に潰す。CRLF ⇄ LF の変換を
        「差分なし」と見なすと、変換とピン更新が同居したコミットが例外 1d に乗る。
      - 不正 UTF-8 を `errors="replace"` で復号すると、異なるバイト列が同じ U+FFFD に
        潰れて「行が一致」に見える。比較はバイトで行い、復号は正規表現に当てる直前だけ。
    有界キャッシュにするのは、再利用が「1 コミットにつき前後 2 本」の局所パターンだけで、
    無制限だと走査したコミットぶんのファイル全文をプロセス寿命のあいだ抱え続けるため。
    """
    proc = git_raw("show", f"{sha}:{path}")
    return proc.stdout if proc.returncode == 0 else None


def decode_preserving(raw: bytes) -> str:
    """git から取ったバイト列（行でもブロブ全体でも）を**往復可能な形で**復号する。

    `errors="replace"` は異なる不正バイトを同じ U+FFFD に潰すので使わない。潰すと、
    復号後に比べる箇所（frontmatter の本文比較・`uses:` 行の owner/repo 比較）で
    別内容が「一致」に見える。surrogateescape なら不正バイトが 1 バイトずつ別の
    サロゲートに写るため、相違が保存される。
    """
    return raw.decode("utf-8", "surrogateescape")


def is_metadata_only_change(sha: str, path: str) -> bool:
    """そのコミットの変更が frontmatter のメタデータだけかを判定する。

    ADR 0073 の移設は sources のパス表記を一斉に書き換えた。本文は 1 文字も変わって
    いないのに「内容変更」と見なすと、それを sources に持つ knowledge が軒並み stale に
    なる（実測 7 件）。文書クラスの付与も同じ形で自己ノイズを生む。
    """
    new_raw, old_raw = blob_at(sha, path), blob_at(f"{sha}^", path)
    if new_raw is None or old_raw is None:
        return False  # 初回追加や親を辿れない場合は判定しない（内容変更として扱う）
    new_text, old_text = decode_preserving(new_raw), decode_preserving(old_raw)
    new_fm, new_body = split_frontmatter(new_text)
    old_fm, old_body = split_frontmatter(old_text)
    if new_fm is None or old_fm is None or new_body != old_body:
        return False
    new_blocks, old_blocks = frontmatter_blocks(new_fm), frontmatter_blocks(old_fm)
    changed = {k for k in set(new_blocks) | set(old_blocks) if new_blocks.get(k) != old_blocks.get(k)}
    return bool(changed) and changed <= METADATA_KEYS


# 例外 1d の対象パス。**ワークフローだけに絞る**。判定は行単位・字面ベースで YAML 構造を
# 見ないので、絞らないと Markdown のコードフェンス内に書いた `uses:` の見本を書き換えただけで
# その文書の stale 検査が消える（README / ADR 0081 も「対象はワークフロー」として書いている）。
RE_WORKFLOW_PATH = re.compile(r"^\.github/workflows/[^/]+\.ya?ml$")

# GitHub Actions の `uses:` 行。サプライチェーン対策でピン留めしている 40 hex を捕まえる。
# group(1)=インデントと `uses:` / group(2)=owner/repo / group(3)=40 hex / group(4)=末尾の版注記。
#   - group(2) は `/` を含まない 2 要素に限る。緩めると再利用可能ワークフロー参照
#     （`owner/repo/.github/workflows/x.yml@<sha>`）まで拾い、呼び先のジョブ構成ごと
#     変わる更新を免除してしまう。
#   - group(4) は**版注記の形だけ**許す。dependabot が hex と一緒に `# v4` → `# v7.0.0` の
#     ように注記も書き換えることがあるため（実例 884f982 = actions/setup-node）。任意の
#     コメントを許すと、注記を無関係な散文へ差し替えた変更まで免除される。`#` の前に空白を
#     必須にするのは、`@<40hex>#v4` は YAML ではコメントにならず ref の一部だから。
# 同一パス内でも `run: |` のブロックスカラに同じ形の行があれば拾ってしまうが、
# 対象をワークフローに絞ってあるので実害は「自リポの CI スクリプト本文」に限られる。
RE_USES_PIN = re.compile(
    r"^(\s*(?:-\s+)?uses:\s+)([^@\s/]+/[^@\s/]+)@([0-9a-fA-F]{40})([ \t]+#[ \t]*v?[0-9][0-9A-Za-z._+-]*)?$"
)


def is_pin_only_change(sha: str, path: str) -> bool:
    """そのコミットの変更が `uses:` のピン留め SHA 更新だけかを判定する（例外 1d）。

    規約と背景は docs/knowledge/README.md の例外 1d と ADR 0081。要点は、ピンの hex が
    上がっても下流 knowledge が語るジョブ構成は変わらないので読み直す理由が無いこと。
    例外 1b では吸収できない——is_metadata_only_change は split_frontmatter に依存しており、
    先頭が `---` でない .yml は常に「内容変更」と判定される。
    """
    if not RE_WORKFLOW_PATH.match(path):
        return False
    new_raw, old_raw = blob_at(sha, path), blob_at(f"{sha}^", path)
    if new_raw is None or old_raw is None:
        return False  # 初回追加や親を辿れない場合は判定しない（内容変更として扱う）
    # `b"\n"` で割る。str の splitlines() は \r や \x0b でも切るので、CRLF 変換や制御文字の
    # 混入が「差分なし」に見えてしまう。行末の \r を残せば正規表現に合わなくなり、内容変更
    # として扱われる（保守側）。
    new_lines, old_lines = new_raw.split(b"\n"), old_raw.split(b"\n")
    if len(new_lines) != len(old_lines):
        return False  # 行の増減はジョブ・ステップの追加削除なので内容変更
    hex_changed = False
    for new_line, old_line in zip(new_lines, old_lines):
        if new_line == old_line:
            continue
        new_pin = RE_USES_PIN.match(decode_preserving(new_line))
        old_pin = RE_USES_PIN.match(decode_preserving(old_line))
        if not new_pin or not old_pin:
            return False
        # 変わってよいのは 40 hex と末尾の版注記だけ。owner/repo の差し替えは別の action を
        # 呼ぶことなのでジョブの意味が変わる＝内容変更。タグ → hex のような形式変更も
        # 片側が RE_USES_PIN に合わないので上で弾かれる。
        if new_pin.group(1, 2) != old_pin.group(1, 2):
            return False
        if new_pin.group(3) != old_pin.group(3):
            hex_changed = True
    # 注記だけを書き換えたコミットは免除しない（「ピン留め SHA 更新のみ」が例外の条件）。
    # hex_changed は差分行の中でしか立たないので、**これだけで規約の条件「差分行が 1 行以上」も
    # 兼ねる**（差分が無ければ立たない）。差分行数を別に数える必要はない。
    return hex_changed


class GitFailed(Exception):
    """git コマンドそのものが失敗した。

    走査の途中で握り潰すと「そのコミットは対象パスを触っていない」と同じ扱いになり、
    最後の内容変更コミットが黙って飛んで stale 検査が静かに通る（fail-open）。上位で
    ScanAborted に変換して error にする。
    """


def path_status(sha: str, path: str) -> "tuple[str | None, str | None]":
    """コミット sha における path の (status, リネーム元) を返す。

    name-status に **パス指定を渡さない**。渡すとリネーム元が絞り込みから外れて対にならず、
    R100 ではなく A（新規追加）として報告される（実測）。コミット全体の name-status を取り、
    対象パスを終点とする行だけを見る。
    core.quotePath=false を明示するのは、既定だと非 ASCII パスが "\\346\\234\\200..." の
    クォート表記になり終点一致が外れて R100 除外が破れるため。

    **失敗を (None, None) に混ぜない。** それは「このコミットは触っていない」と同義で、
    走査を続けると検査が黙ってスキップされる。

    **マージに対する `git show` の既定＝combined diff（`--cc`）は契約**（ADR 0084）。
    `--cc` は「**全ての親と異なる**パス」を列挙するので、evil merge（マージ自身だけが内容を
    変える形）がここで拾える。壊し方は 2 種類あり、**落ちるテストが違う**（実測）:

      - **マージが無出力になる変更**（`git diff-tree`（`-c` 無し）/ `--diff-merges=off`）
        → stale 検査に恒久的な穴が開く。
        `test_evil_merge_is_detected_as_content_change` が落ちる。
      - **第 1 親比較へ変える変更**（`--first-parent` / `-m`）→ **無出力にはならない**
        （第 1 親との差分が出るので evil merge は依然見える）。壊れるのは対象集合の方で、
        片親を採るマージまで「内容変更」に化けて偽 STALE を量産する。
        `git show` 側なら `test_rename_inside_merge_is_treated_as_content_change`、
        `git log` 側なら `test_merge_taking_one_side_is_attributed_to_ancestor` が落ちる。

    **既知の限界**: combined diff はリネームを `RR` として出し `R100` にならないので、
    **マージ内での純粋なリネームは免除が効かず偽の STALE になる**（`-M100%` はマージに
    効かず、リネーム元も取れない）。fail-closed 側なので塞いでいない（ADR 0084 決定 3・
    `test_rename_inside_merge_is_treated_as_content_change` が現状の挙動を pin している）。
    """
    proc = git(
        "-c", "core.quotePath=false", "show", "--format=", "--name-status", "-M100%", sha
    )
    if proc.returncode != 0:
        raise GitFailed(f"git show --name-status が失敗した（{proc.stderr.strip()[:200]}）")
    for line in proc.stdout.splitlines():
        parts = line.split("\t")
        if len(parts) >= 2 and parts[-1] == path:
            status = parts[0]
            src = parts[1] if status.startswith("R") and len(parts) >= 3 else None
            return status, src
    return None, None


class ScanAborted:
    """`last_content_change` が走査を完遂できなかったことを表す番兵。

    **SHA 文字列とは型で区別する。** 素の str にすると、呼び出し側が `is None` だけ見て
    番兵を SHA として `merge-base --is-ancestor` へ渡し、偽の STALE を出す事故が起きうる。
    `reason` は error 文言にそのまま載るので、原因（ページ予算 / リネーム予算 / git の失敗）を
    取り違えて無関係な定数をいじらせないための情報を入れる。
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

    次の 3 種類は「内容変更ではない」として遡る（規約は docs/knowledge/README.md の例外 1 / 1b / 1d）:
      - R100（内容差分ゼロのリネーム）。ディレクトリ移設で全件が stale になるのを防ぐ
      - frontmatter のメタデータだけの変更（sources のパス追従・doc_class 付与など）
      - `uses:` のピン留め SHA 更新だけの変更（dependabot の Actions 更新 PR）

    **`--follow` は使わない。** `--follow` はリネームで履歴を打ち切ることがあり（実測:
    ADR 0036 は移設コミット 1 件しか返さず、それ以前の起票コミットへ辿れなかった）、
    そこで打ち切られると「履歴を辿れない＝判定不能」に落ちる。代わりに、R100 を見つけたら
    **そのコミットの親からリネーム元のパスで履歴を取り直す**。リネームが何段重なっても効く。

    **窓（limit）の使い切りと履歴の尽きを混同しない。** 呼び出し側は None を warning に
    落として stale 判定をスキップする（fail-open）ので、「窓の中が全部除外対象だった」だけで
    None を返すと、除外対象のコミットを積むほど検査が消える経路になる。例外 1d で機械が
    量産するコミットが除外対象になった以上これは現実的なので、**取れた件数が limit に達して
    いたら次のページへ進む**。

    戻り値は 3 通り。**「走査を完遂できなかった」を「履歴が無い」に混ぜないこと**が要点で、
    混ぜると検査側の都合が warning に落ちて同じ fail-open が一段外側で再現する。

      - SHA         : 内容が最後に変わったコミット
      - None        : **履歴が無い**（未コミット・履歴が尽きた・shallow）→ 呼び出し側は warning
      - ScanAborted : **走査を完遂できなかった**（ページ予算 / リネーム予算 / git の失敗）
                      → 呼び出し側は error

    `max_pages` は**走査全体**のページ数（リネームを辿っても取り直さない）。パス単位にすると
    実際の上限が `max_renames × max_pages` に膨らみ、宣言した値と乖離するうえ、病的な履歴では
    `adr` ジョブの timeout が先に来て「打ち切りを error にする」意図が届かない。
    `max_renames=N` のとき実際に辿れるリネームは **N-1 段**（N 段目を見つけた時点で打ち切る）。
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
            # git 側の失敗は環境の都合ではなく検査が回っていないこと。warning に落とすと
            # その source の stale 判定が黙って消える。
            return ScanAborted(f"git log が失敗した（{proc.stderr.strip()[:200]}）")
        shas = proc.stdout.split()
        if not shas:
            return None  # 履歴が尽きた
        renamed = False
        for sha in shas:
            status, rename_src = path_status(sha, current)
            if status is None:
                # **防御的な分岐で、実際には到達しない**（ADR 0084 で計測。実リポジトリの
                # 全 `sources`・全履歴でも 183 ケースのテストでも 0 回）。`git log` の既定の
                # 単純化は「辿る親に対して当該パスを変えたコミット」しか返さず、`git show` の
                # `--cc` は「全親と異なるパス」を必ず出すので、両者が食い違わないため。
                #   - 片親と同じ内容になったマージは `git log` が**そもそも列挙しない**
                #     （TREESAME な親を辿る）。ここへは来ない。
                #   - evil merge（全親と異なる）は `--cc` が列挙するので status が付く。
                #     ADR 0081 が「既知の限界」として記録した「恒久的に不可視」は誤りだった。
                # 到達しないと分かっていても落とさないのは、`git log` 側のオプションを足すと
                # （`--full-history` 等）前提が崩れるため。**この分岐を「内容変更」に変える
                # 変異はどのテストでも落ちない**——テストで守れない代わりに、ここに理由を書く。
                continue
            if status == "R100":
                if not rename_src:
                    return sha  # リネーム元を取れない＝判定できないので内容変更扱い
                current = rename_src
                tip = f"{sha}^"
                skip = 0  # パスが変わったので窓の位置は取り直す（ページ予算は全体で数える）
                renames += 1
                renamed = True
                break
            # ワークフローの判定を先に置く。免除に当たったときに frontmatter の分解を
            # 試みる無駄が消える（.md 側は RE_WORKFLOW_PATH で即 False になる）。
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


# --- REQ（要件 ID）の検査 ---------------------------------------------------
# 規約は docs/knowledge/README.md「REQ-ID（要件 ID）の規約」。表の位置を見出し構造に
# 依存させないため、範囲はマーカーで宣言する（doc-classes.md の表と同じ方式）。
RE_REQ_BEGIN = re.compile(r"^<!--\s*REQ:begin\s+(D\d{2})\s*-->$")
RE_REQ_END = re.compile(r"^<!--\s*REQ:end\s+(D\d{2})\s*-->$")
# 「マーカーのつもりで書かれた行」を緩く拾う。厳密形（D + 2 桁）から外れた綴りを
# 黙って地の文として扱うと、begin と end が揃って外れたときに REQ 表が丸ごと
# 無検査になる（実測: `<!-- REQ:begin D1 -->` で表全体が素通りする fail-open）。
RE_REQ_MARKER_LOOSE = re.compile(r"^<!--\s*REQ\s*:\s*(?:begin|end)\b.*-->$", re.IGNORECASE)
# ブロック外に取り残された表の検出用。REQ-ID の厳密形か見出し行だけを見る——
# 地の文が REQ-ID に言及するだけ（`REQ-D01-004 を参照`）で落とすと使い物にならない。
RE_REQ_ID_ANY = re.compile(r"REQ-D\d{2}-\d{3}")
RE_REQ_HEADER = re.compile(r"^\|?\s*REQ-ID\s*\|")
RE_FENCE = re.compile(r"^(`{3,}|~{3,})")
# 引用（blockquote）の行頭記号。剥がしてから判定しないと、`> ` を付けるだけで
# マーカーも表も全部の検査を素通りする（GitHub は引用内でも表として描画する）。
RE_BLOCKQUOTE = re.compile(r"^(?:>\s*)+")
RE_REQ_ID = re.compile(r"^REQ-(D\d{2})-(\d{3})$")
# タイトル付き `[x](path "題")` と山括弧付き `[x](<path>)` も拾う。素朴な
# `\(([^)\s]+)\)` だとタイトルを付けるだけで実在検査・絶対パス拒否を全部迂回できる。
RE_MD_LINK = re.compile(r"""\[[^\]]*\]\(\s*<?([^)\s>]+)>?(?:\s+[^)]*)?\)""")
# インラインコード。検証手段の列にはコマンドを書くので、その中のリンク様文字列
# （`grep '[x](nope.md)' file` 等）を実リンクとして検査すると偽陽性になる。
# **改行を跨がせない**。本文を 1 文字列で走査するので、散文中の単独バッククォートが
# 次のインラインコードと対になり、その間のリンクを黙って飲み込む（実測で再現した
# fail-open。検査の目的そのものを裏切る）。
RE_INLINE_CODE = re.compile(r"`[^`\n]*`")
RE_TABLE_SEP_CELL = re.compile(r"^:?-+:?$")
# スキーム付き URI（http / https / mailto / ftp / tel …）。リンク検査の対象外。
RE_URI_SCHEME = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:")

REQ_COLUMNS = ("REQ-ID", "要件", "検証手段", "出典", "status")
# Confirmed / Tentative / Conflict は frontmatter の status と同義。Retired は
# 「かつて要件だったが取り下げた」——番号を再利用しないため行は残す。
REQ_STATUSES = ("Confirmed", "Tentative", "Conflict", "Retired")
# 「空」と見なすセル表記。空欄だけを弾くと `-` や TBD で素通りし、検証手段の無い
# Confirmed（＝願望と区別が付かない要件）を通してしまう。
EMPTY_CELLS = {"", "-", "–", "—", "tbd", "unknown", "n/a", "未定", "なし", "未整備"}


def split_row(line: str) -> list[str]:
    """GFM の表行をセルへ分割する。

    エスケープされたパイプ（`\\|`）では割らない。検証手段の列にはコマンドを書くので、
    素朴に `split("|")` すると `... | jq` のような正当なセルが「列数が 5 でない」に化ける。
    """
    body = line.strip()
    if body.startswith("|"):
        body = body[1:]
    if body.endswith("|") and not body.endswith(r"\|"):
        body = body[:-1]
    return [c.strip().replace(r"\|", "|") for c in re.split(r"(?<!\\)\|", body)]


def looks_like_req_row(line: str) -> bool:
    """表の行として **第 1 セルが REQ-ID** かを見る。

    地の文が REQ-ID に言及するだけ（``REQ-D01-004 の検証は `cmd | tail -1` で行う``）や、
    REQ-ID を右側の列で参照するトレーサビリティ表（`| 予想 | REQ-D01-004 |`）を
    落とさないための絞り込み。REQ 表は第 1 列が REQ-ID と規約で決まっているので、
    「第 1 セルがちょうど REQ-ID」だけを REQ 表の行と見なす。
    """
    if "|" not in line:
        return False
    cells = split_row(line)
    return bool(cells) and RE_REQ_ID.match(cells[0]) is not None


def parse_req_blocks(text: str) -> "tuple[list[tuple[str, list[list[str]]]], list[str]]":
    """REQ ブロックを (クラス, 表の行) のリストに切り出す。第 2 戻り値は構造エラー。

    マーカーの対応崩れは「REQ 表が丸ごと検査対象から消える」経路（fail-open）なので、
    黙って無視せず error として返す。塞ぐ経路は 4 つ:

      1. マーカーを付け忘れた REQ 表      → ブロック外の REQ 行を検出する
      2. マーカーの綴り違い（`D1` 等）    → 緩い正規表現で拾って書式不正にする
      3. コードフェンスの閉じ忘れ         → 走査終了時に開いたままなら error
      4. begin / end の対応崩れ           → 従来どおり error

    コードフェンス内のマーカー・表は規約の説明文が持つため無視する（同種・同長以上の
    フェンスでのみ閉じる＝``` の中の ~~~ で誤って閉じない）。
    """
    blocks: list[tuple[str, list[list[str]]]] = []
    errors: list[str] = []
    cls: "str | None" = None
    rows: list[list[str]] = []
    lines, unclosed_fence = unfenced_lines(text)
    for lineno, stripped in lines:
        begin = RE_REQ_BEGIN.match(stripped)
        end = RE_REQ_END.match(stripped)
        if not begin and not end and RE_REQ_MARKER_LOOSE.match(stripped):
            errors.append(
                f"{lineno} 行目: REQ マーカーの書式が不正 → {stripped}"
                "（<!-- REQ:begin D<NN> --> / <!-- REQ:end D<NN> --> のみ）"
            )
            continue
        if begin:
            if cls is not None:
                errors.append(f"REQ ブロック（{cls}）が閉じられないまま次の begin がある")
                blocks.append((cls, rows))
            cls, rows = begin.group(1), []
            continue
        if end:
            if cls is None:
                errors.append(f"対応する begin の無い REQ:end がある（{end.group(1)}）")
            else:
                if end.group(1) != cls:
                    errors.append(
                        f"REQ ブロックの begin({cls}) と end({end.group(1)}) でクラスが違う"
                    )
                blocks.append((cls, rows))
            cls, rows = None, []
            continue

        if cls is not None:
            if stripped.startswith("|"):
                rows.append(split_row(stripped))
            elif RE_REQ_ID_ANY.search(RE_INLINE_CODE.sub(" ", stripped)):
                # GFM は行頭パイプを省いても表になる。黙って捨てると、その要件が
                # 一意性・status・空セルの検査から丸ごと消える（重複が通る）。
                errors.append(
                    f"{lineno} 行目: REQ ブロック内に表の行として読めない REQ 行がある"
                    f" → {stripped}（行頭を `|` で始める）"
                )
        elif RE_REQ_HEADER.match(stripped) or looks_like_req_row(stripped):
            errors.append(
                f"{lineno} 行目: REQ 表がマーカーの外にある → {stripped}"
                "（マーカーで囲まないと一意性・status の検査から丸ごと漏れる）"
            )

    if unclosed_fence:
        errors.append("コードフェンスが閉じられていない（以降の REQ 表が無検査になる）")
    if cls is not None:
        errors.append(f"REQ ブロック（{cls}）が閉じられていない")
        blocks.append((cls, rows))
    return blocks, errors


def unfenced_lines(text: str) -> "tuple[list[tuple[int, str]], bool]":
    """フェンス外の (行番号, blockquote を剥がした行) と、未閉じフェンスの有無を返す。

    コードフェンスは同種かつ**同長以上**でのみ閉じる（GFM）。長さを見ないと、```` で
    囲んで ``` の見本を書く定番の書き方で内側が誤って閉じ、見本が実データに化ける。
    REQ 表の走査と本文リンクの走査で**判定規則を 2 箇所に持たない**ようここへ集約する。
    blockquote を剥がすのは、`> ` を付けるだけで全検査を素通りさせないため。
    """
    kept: list[tuple[int, str]] = []
    fence: "tuple[str, int] | None" = None
    for lineno, line in enumerate(text.splitlines(), 1):
        stripped = RE_BLOCKQUOTE.sub("", line.strip())
        opener = RE_FENCE.match(stripped)
        if opener:
            token = opener.group(1)
            if fence is None:
                fence = (token[0], len(token))
            elif token[0] == fence[0] and len(token) >= fence[1]:
                fence = None
            continue
        if fence is None:
            kept.append((lineno, stripped))
    return kept, fence is not None


def check_body_links(
    rel: str,
    root: Path,
    text: str,
    errors: list[str],
    link_seen: "set[str] | None" = None,
    report_unclosed: bool = False,
) -> None:
    """frontmatter を除いた本文のリンクを、行番号付きで検査する。

    **行ごとに切って走査しない**。`[ラベルが\n改行を跨ぐ](path.md)` のような正当な
    Markdown が黙って無検査になる（1 巡目に潰した「改行跨ぎのインラインコードが
    リンクを飲み込む」と同種の silent-green 経路）。フェンス外の行を連結して
    まとめて走査し、マッチ位置から行番号を引く。
    """
    fm, body = split_frontmatter(text)
    # frontmatter のぶんだけ行番号がずれるので足し戻す（`---` 2 行 + 中身）。
    offset = 0 if fm is None else len(fm.splitlines()) + 2
    lines, unclosed = unfenced_lines(body)
    if unclosed and report_unclosed:
        # 走査対象の文書は parse_req_blocks が同じ error を出すので二重に出さない。
        # README / CLAUDE.md はこの経路しか通らず、閉じ忘れ以降のリンクが丸ごと
        # 無検査のまま exit 0 になる（本 PR が潰している silent-green と同型）。
        errors.append(f"{rel}: コードフェンスが閉じられていない（以降のリンクが無検査になる）")
    if not lines:
        return
    joined = "\n".join(line for _, line in lines)
    # 連結後のオフセット → 元の行番号。インラインコードの除去は行内で完結する
    # （改行を跨がない正規表現）ので、置換後も行の対応はずれない。
    starts: list[int] = []
    pos = 0
    for _, line in lines:
        starts.append(pos)
        pos += len(line) + 1

    def label_at(index: int) -> str:
        i = bisect.bisect_right(starts, index) - 1
        return f"本文（{lines[max(i, 0)][0] + offset} 行目）の"

    check_links(rel, root, joined, errors, seen=link_seen, label_at=label_at)


def case_exact(path: Path, root: Path) -> bool:
    """root から path までの**全成分**が実在の名前と大文字小文字まで一致するかを見る。

    macOS(APFS) は大文字小文字を区別しないので `exists()` が通り、Linux の CI だけが
    落ちる。最終成分だけ見るとディレクトリ名の大小違い（`../Specifications/x.md`）が
    素通りするので、1 階層ずつ照合する。
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
            return True  # 読めないディレクトリは判定しない（他の検査に委ねる）
        current = current / part
    return True


# 成功行に出す「実在を検査したリンク数」。スキーム付き URI・アンカー・同一先の重複は
# 数えない（#604 (c) の「まず数える」を後から同じ定義で再現するためのベースライン）。
LINK_COUNT = 0


def is_external_link(target: str) -> bool:
    """リポジトリ内のパスとして解決すべきでないリンク先か。

    スキーム付き URI（http/https/mailto に限らず ftp・tel 等）と protocol-relative、
    同一文書内アンカーが該当する。ホワイトリストにすると新しいスキームを足すたびに
    誤検知が出るので、スキームの有無で判定する。呼び出し元は iter_links だけ。
    """
    return target.startswith(("#", "//")) or bool(RE_URI_SCHEME.match(target))


def repo_relative_path_error(raw: str) -> "str | None":
    """`sources` / orphan 例外表に書けるパスでない理由を返す（正常なら None）。

    **`sources` と例外表で同じ 1 本を通す。** 片方だけにガードがあると、同じ形式を
    要求しているのに一方だけ非正規形・リポジトリ外を素通りさせ、後段の突合で
    「ADR ではない」のような**事実と逆の診断**が出る（実測）。

    正規形の判定は **`Path(...).as_posix()` の語彙的正規化だけ**で行う——`os.path.normpath`
    や `Path.resolve()` を使うと `..` まで畳んでしまい、直前の「`..` を拒否する」判定が
    静かに効かなくなる（`docs/original-docs/../../etc/hosts` が通る）。
    """
    if raw.startswith("/") or ".." in Path(raw).parts:
        # 絶対パスや .. を許すと Path(root) / "/etc/hosts" が root を捨てて外を指し、
        # 「由来はリポジトリ内で辿れる」という検査の前提が崩れる。
        return "リポジトリ相対パスで書く（絶対パス・`..` は使わない）"
    if Path(raw).as_posix() != raw:
        # `./docs/...` や `docs//x.md` は **実在検査だけ通って stale 判定から静かに外れる**。
        # path_status が git show --name-status の出力と終点一致で突き合わせるため、
        # `./` 付きは永久に一致せず「履歴を辿れず」の warning に退化する（実測）。
        # 突合用に正規化した集合を別に持つ手もあるが、それだと「どちらの形式か」を持つ
        # 場所が増える——ADR 0083 が例外表のパス形式で却下したのと同じ理由で、
        # **形式を 1 つに強制する**方を採る。
        return "正規形で書く（`./` や重複スラッシュを使わない）"
    return None


def iter_links(fragment: str) -> "Iterator[tuple[int, str, str]]":
    """`fragment` の Markdown リンクを (マッチ開始位置, 生のリンク先, アンカーを剥がしたパス) で返す。

    **リンクを拾う検査はすべてこの 1 本を通す**（check_links / repo_relative_targets）。
    インラインコードのマスク・`RE_MD_LINK` の走査・外部リンクの除外・アンカー剥がしを
    2 か所に書くと、リンク書式を足したときに片方だけ直す事故が起きる——「実在は検査したが
    sources 突合はしていない」という非対称が静かに生まれる。

    **実在するか / どれを error にするかは呼び出し側の責務**（そこは意図して非対称:
    check_links は絶対パス・リポジトリ外・実在しないを error にしてディレクトリを許す。
    repo_relative_targets は `sources` に載せうるもの＝実在するファイルだけを黙って選ぶ）。

    インラインコードは**長さを保存して**マスクする。除去すると後続のマッチ位置が前へずれ、
    呼び出し側が引く行番号が過少になる（実測で 223 本中 210 本が誤り）。
    """
    masked = RE_INLINE_CODE.sub(lambda m: " " * len(m.group(0)), fragment)
    for matched in RE_MD_LINK.finditer(masked):
        target = matched.group(1)
        # スキーム付き URI・protocol-relative・同一文書内アンカーはリポジトリ内のパスでない。
        if is_external_link(target):
            continue
        cleaned = target.split("#", 1)[0]
        if not cleaned:
            continue
        yield matched.start(), target, cleaned


def repo_relative_targets(rel: str, root: Path, fragment: str) -> "list[str]":
    """`fragment` の Markdown リンクのうち、リポジトリ内の**実在するファイル**をルート相対で返す。

    `sources` との突合用。`sources` はリポジトリルート相対、本文・REQ 表のリンクは
    文書からの相対なので、**比較の前に必ずここを通して基準を揃える**（ADR 0083）。

    **実在するファイルだけを返す。** リンク切れ・リポジトリ外・絶対パスは check_links が
    別の error として報告する担当で、ここで拾うと 1 本のリンク切れに対して
    「実在しない」と「sources に無い」の 2 件が出る。しかも `sources` は実在するファイルしか
    受け付けない（検査 4）ので、「sources に足せ」は壊れたリンクへの助言として誤り。
    """
    out: list[str] = []
    for _, _, cleaned in iter_links(fragment):
        if cleaned.startswith("/"):
            continue  # 絶対パスは check_links が error にする
        resolved = ((root / rel).parent / cleaned).resolve()
        if not resolved.is_file():
            continue
        try:
            out.append(resolved.relative_to(root.resolve()).as_posix())
        except ValueError:
            continue  # リポジトリ外
    return out


def check_links(
    rel: str,
    root: Path,
    fragment: str,
    errors: list[str],
    *,
    label: str = "",
    seen: "set[str] | None" = None,
    label_at: "Callable[[int], str] | None" = None,
) -> None:
    """`fragment` に含まれる Markdown リンクの実在を検査する。

    REQ 表の出典・検証手段と本文の双方から呼ぶ。`seen` を渡すと同じリンク先を
    二重に報告しない（REQ 表の行は本文にも含まれるため、渡さないと同じ 1 本の
    リンク切れが 2 件の error になる）。`label_at` を渡すと、マッチ位置から
    ラベル（本文なら行番号入り）を作る。
    """
    # リンクの抽出・外部リンクの除外・アンカー剥がしは iter_links に集約している
    # （sources 突合と同じ 1 本を通す。片方だけ書式を足す事故を防ぐため）。
    global LINK_COUNT
    for start, target, cleaned in iter_links(fragment):
        here = label_at(start) if label_at else label
        # 同じファイルへのリンクは #anchor 違いでも 1 件に畳む（同じ欠落を何度も報告しない）。
        if seen is not None:
            if cleaned in seen:
                continue
            seen.add(cleaned)
        LINK_COUNT += 1  # ここから先が実在検査。skip / 重複は数えない
        # sources 検査（main）と同じ理由でリポジトリ外を弾く。絶対パスを許すと
        # Path 連結が左辺を捨てて外を指し、「参照先はリポジトリ内で辿れる」という
        # 前提が崩れる（`[外](/etc/hosts)` が実在扱いで通る）。
        if cleaned.startswith("/"):
            errors.append(f"{rel}: {here}リンクは文書からの相対パスで書く → {target}")
            continue
        resolved = ((root / rel).parent / cleaned).resolve()
        if not resolved.is_relative_to(root.resolve()):
            errors.append(f"{rel}: {here}リンクがリポジトリ外を指している → {target}")
        elif not resolved.exists():
            # ディレクトリへの相対リンクも正当（README が `scripts/` 等への
            # リンクを誘導している）。sources は「1 ファイル＝1 出典」なので非対称。
            errors.append(f"{rel}: {here}リンク先が実在しない → {target}")
        elif not case_exact(resolved, root.resolve()):
            # macOS(APFS) は大文字小文字を区別しないので exists() が通る。Linux の CI
            # だけが落ちる非対称を pre-push の時点で潰す。
            errors.append(f"{rel}: {here}リンク先の大文字小文字が実ファイルと違う → {target}")


def check_req_blocks(
    rel: str,
    text: str,
    doc_classes: "list[str] | None",
    declared: dict,
    seen: "dict[str, str]",
    root: Path,
    errors: list[str],
    sources: "set[str]",
    link_seen: "set[str] | None" = None,
) -> None:
    """1 文書ぶんの REQ 表を検査する。`seen` は REQ-ID → 初出パスの全体台帳。

    `sources` はこの文書の frontmatter の `sources`（ルート相対）。出典セルが名指しした
    一次資料がここに載っているかを突き合わせる（検査 11）。

    唯一の参照元からその ADR を落とすと、検査 11 と orphan 検査（12）の両方が鳴る。
    **この重複は意図的**——言っていることが違う（「この文書が根拠を watch していない」と
    「誰もこの ADR を watch していない」）。片方を抑止すると、参照元が複数ある場合に
    orphan 側が鳴らない事実が見えなくなる。リンク切れの二重報告
    （repo_relative_targets の docstring）とは別の話で、あちらは同じ 1 つの欠陥の重複だった。

    突合の対象は `出典` 列だけで、`検証手段` 列は見ない（そこに挙がるのは測り方であって
    蒸留元ではない・ADR 0083 の意図的スコープ）。
    """
    blocks, structural = parse_req_blocks(text)
    # 同じ出典を複数の REQ が挙げていても、未収載の報告は文書内で 1 回に畳む。
    origin_reported: set[str] = set()
    for msg in structural:
        errors.append(f"{rel}: {msg}")

    for cls, rows in blocks:
        if cls not in declared:
            errors.append(f"{rel}: REQ ブロックのクラス {cls} は {REGISTRY} に定義が無い")
        elif not doc_classes or cls not in doc_classes:
            # 他クラスの要件を勝手に抱え込ませない（番号空間の持ち主を文書クラスで固定する）。
            errors.append(
                f"{rel}: REQ ブロックのクラス {cls} がこの文書の doc_class に含まれていない"
                f"（doc_class={doc_classes}）"
            )

        # 見出し行は必須で、列名も順序も固定。位置で意味付けして読む（下の unpack）ので、
        # 順序が入れ替わると「Confirmed には検証手段が必須」の検査が別の列に当たる。
        if len(rows) < 2 or rows[0] != list(REQ_COLUMNS):
            errors.append(
                f"{rel}: REQ ブロック（{cls}）の見出し行が "
                f"`| {' | '.join(REQ_COLUMNS)} |` でない"
            )
            continue
        if len(rows[1]) != len(REQ_COLUMNS) or not all(
            RE_TABLE_SEP_CELL.match(c) for c in rows[1]
        ):
            errors.append(
                f"{rel}: REQ ブロック（{cls}）の区切り行が {len(REQ_COLUMNS)} 列になっていない"
            )
            continue

        # 空セルだけの行も落とさない（落とすと有効行に紛れた 1 本が無言で消える）。
        data = [cells for cells in rows[2:] if cells]
        if not data:
            errors.append(f"{rel}: REQ ブロック（{cls}）に要件行が 1 つも無い")
            continue

        for cells in data:
            if len(cells) != len(REQ_COLUMNS):
                errors.append(
                    f"{rel}: REQ 表の列数が {len(REQ_COLUMNS)} でない"
                    f"（{' | '.join(REQ_COLUMNS)}）→ {' | '.join(cells)}"
                )
                continue
            req_id, requirement, verification, origin, status = cells

            matched = RE_REQ_ID.match(req_id)
            if not matched:
                errors.append(f"{rel}: REQ-ID の形式が不正 → {req_id}（REQ-D<NN>-<NNN>）")
                continue
            if matched.group(1) != cls:
                errors.append(
                    f"{rel}: {req_id} のクラスが REQ ブロック（{cls}）と一致しない"
                )
            if req_id in seen:
                errors.append(
                    f"{rel}: {req_id} が重複している（初出 {seen[req_id]}）。"
                    "番号は文書をまたいでクラス内で一意・再利用しない"
                )
            else:
                seen[req_id] = rel

            if requirement.lower() in EMPTY_CELLS:
                errors.append(f"{rel}: {req_id} の要件が空")
            if origin.lower() in EMPTY_CELLS:
                errors.append(f"{rel}: {req_id} の出典が空（由来を辿れない）")
            if status not in REQ_STATUSES:
                errors.append(
                    f"{rel}: {req_id} の status が不正 → {status}"
                    f"（{' / '.join(REQ_STATUSES)}）"
                )
            elif status == "Confirmed" and verification.lower() in EMPTY_CELLS:
                errors.append(
                    f"{rel}: {req_id} は検証手段が空のまま Confirmed になっている"
                    "（測り方の決まっていない要件は Confirmed にできない）"
                )

            # 出典・検証手段が指すリポジトリ内リンクの実在。frontmatter の sources と
            # 同じ理由——由来を辿れないなら要件の根拠を確認できない。セルごとに
            # 渡すのは、インラインコードの除去をセルを跨いで行うと閉じていない
            # バッククォートが隣のセルと対になって本物のリンクを消すため。
            for fragment in (origin, verification):
                check_links(
                    rel, root, fragment, errors, label=f"{req_id} の", seen=link_seen
                )

            # (11) 出典が名指しした一次資料は frontmatter の sources にも載っていること。
            # stale 検査は sources に**挙がっている**行しか見ないので、載せ忘れると
            # 「本文では根拠に挙げたのに、その根拠が更新されても誰も気づかない」状態に
            # なる（ADR 0083）。sources から行を消せば stale も消えるという穴の片側。
            for src in repo_relative_targets(rel, root, origin):
                # 兄弟の knowledge / specifications への相互リンクは蒸留元ではないので
                # 対象外。載せると sources の意味が壊れ、無関係な理由で stale が発火する。
                if not src.startswith(f"{ORIGINAL_DOCS}/"):
                    continue
                if src in sources or src in origin_reported:
                    continue
                origin_reported.add(src)
                errors.append(
                    f"{rel}: {req_id} の出典 {src} が frontmatter の sources に無い"
                    "（出典は減らさず sources 側を足す）"
                )


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

    global _ROOT
    _ROOT = repo_root()
    root = _ROOT
    registry_path = root / REGISTRY
    if not registry_path.is_file():
        print(f"クラス定義が見つからない: {REGISTRY}", file=sys.stderr)
        return 1

    errors: list[str] = []
    warnings: list[str] = []
    # 「違反があるから落とす」（--warn-only で抑止できる）と「検査そのものが成立していない
    # から落とす」（抑止できない）の二分法をコード上で明示する。後者はマーカー欠落
    # （extract_block の sys.exit）と ADR 0 件。
    fail_closed = False

    # --- レジストリを読む ---
    registry_text = registry_path.read_text(encoding="utf-8")
    declared: dict[str, dict] = {}
    for line in extract_block(registry_text, "doc-classes"):
        stripped = line.strip()
        row = RE_CLASS_ROW.match(stripped)
        if row:
            declared[row.group(1)] = {"state": row.group(3), "count": int(row.group(4))}
        elif RE_NA_ROW.match(stripped):
            # クラス行に見えるのに書式が崩れている。黙って落とすとそのクラスが「未定義」に
            # なり、参照している文書が全部 error になって原因が分からなくなる。
            errors.append(f"{REGISTRY}: クラス一覧の書式が崩れている行がある → {stripped}")
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

    # 割当索引を先に読む。走査後に読むと、マーカー欠落の sys.exit がそれまでに
    # 溜めた errors / warnings を捨てて落ちる（重い git 走査も無駄になる）。
    indexed: dict[str, str] = {}
    for line in extract_block(registry_text, "doc-classes-index"):
        stripped = line.strip()
        if not stripped.startswith("|"):
            continue
        cells = split_row(stripped)
        if len(cells) != 2:
            errors.append(f"{REGISTRY}: 割当索引の書式が崩れている行がある → {stripped}")
            continue
        if cells[0] == "文書" or RE_TABLE_SEP_CELL.match(cells[0]):
            continue
        if cells[0] in indexed:
            # 後勝ちで上書きすると、片方が実態とズレていても無言で通る。
            errors.append(f"{REGISTRY}: 割当索引に {cells[0]} の行が 2 つある")
        indexed[cells[0]] = cells[1]

    # orphan ADR 検査（12）の例外。理由必須の 2 列表で、パスは sources と同じリポジトリ
    # ルート相対（比較相手に形式を合わせる・ADR 0083）。スクリプト内の定数ではなく
    # レジストリに置くのは、例外を増やす行為を文書レビューに乗せるため。
    orphan_exceptions: dict[str, str] = {}
    header_seen = False
    for line in extract_block(registry_text, "adr-orphan-exceptions"):
        stripped = line.strip()
        if not stripped.startswith("|"):
            continue
        cells = split_row(stripped)
        if len(cells) != 2:
            errors.append(
                f"{REGISTRY}: orphan 例外表の書式が崩れている行がある → {stripped}"
                "（2 列。セル内の `|` は `\\|` でエスケープする）"
            )
            continue
        if RE_TABLE_SEP_CELL.match(cells[0]):
            continue
        if not header_seen:
            # **先頭のデータ行は見出しでなければならない。** 見出しを別の列名にすると、
            # その行が「例外エントリ」として読まれ「ファイルが実在しない」という
            # 原因の読めない error になる（実測）。列名は checker がパースする契約。
            header_seen = True
            if cells != list(ORPHAN_EXCEPTION_COLUMNS):
                errors.append(
                    f"{REGISTRY}: orphan 例外表の見出し行が "
                    f"`| {' | '.join(ORPHAN_EXCEPTION_COLUMNS)} |` でない → {stripped}"
                )
            continue
        if not cells[0]:
            # 空セルを黙って通すと `orphan 例外表の  は…` とパスが空欄のまま報告され、
            # どの行のことか分からなくなる。
            errors.append(f"{REGISTRY}: orphan 例外表に ADR 列が空の行がある → {stripped}")
            continue
        # **パスの形式 → 置き場所の順に見る。** 逆にすると `./docs/original-docs/...` が
        # 「配下のパスで書く」と報告され、直すべき点（`./`）が分からない。
        path_error = repo_relative_path_error(cells[0])
        if path_error:
            # sources と同じ文字列で突合するので、非正規形やリポジトリ外は比較から外れる。
            # 黙って落とすと「ADR ではない」という事実と逆の診断が出る（実測）。
            errors.append(f"{REGISTRY}: orphan 例外表のパスは{path_error} → {cells[0]}")
            continue
        if not cells[0].startswith(f"{ORIGINAL_DOCS}/"):
            # ADR 列にパス以外（見出しの貼り直し・散文行）が来たとき、そのまま突合へ流すと
            # 「ファイルが実在しない」という原因の読めない error になる（実測）。
            if cells[0] == ORPHAN_EXCEPTION_COLUMNS[0]:
                errors.append(
                    f"{REGISTRY}: orphan 例外表に見出し行が 2 つある"
                    "（マーカーブロック内の表は 1 つ）"
                )
            else:
                errors.append(
                    f"{REGISTRY}: orphan 例外表の ADR 列は {ORIGINAL_DOCS}/ 配下のパスで書く"
                    f" → {cells[0]}"
                )
            continue
        if cells[0] in orphan_exceptions:
            errors.append(f"{REGISTRY}: orphan 例外表に {cells[0]} の行が 2 つある")
        if cells[1].lower() in EMPTY_CELLS:
            # 理由の無い例外は「なぜ写せないのか」を失うので、次に見た人が消してよいか
            # 判断できない。N/A 宣言表が理由列を必須にしているのと同じ理屈。
            errors.append(f"{REGISTRY}: orphan 例外表の {cells[0]} に理由が書かれていない")
        orphan_exceptions[cells[0]] = cells[1]

    # --- 対象ファイルを走査 ---
    shallow = is_shallow()
    targets: list[Path] = []
    for d in TARGET_DIRS:
        targets.extend(sorted(p for p in (root / d).glob("*.md") if p.name not in EXCLUDED_ENTIRELY))
        # glob は非再帰。サブディレクトリに .md を置かれると無検査域になるので可視化する
        # （現状 docs/specifications/diagrams/ に .md は無い）。
        nested = sorted(p.relative_to(root).as_posix() for p in (root / d).glob("*/**/*.md"))
        for n in nested:
            # #580 で stale を warning → error に上げたのと同じ理由で error にする（ADR 0083）。
            # 文書を 1 階層下げるだけで frontmatter 系・stale・REQ の一意台帳・sources の
            # 被参照が**丸ごと**外れ、警告 1 行のまま exit 0 になる（実測）。orphan 検査が
            # 入った今は「その文書の sources が数えられない」＝他の ADR の誤判定にも波及する。
            # 導入時点で該当 0 件（docs/specifications/diagrams/ に .md は無い）。
            errors.append(
                f"{n}: サブディレクトリの .md は検査対象外（直下に置く）。"
                "この文書は doc_class も sources も stale も一切検査されず、"
                "sources は orphan 検査の被参照にも数えられない"
            )

    # 全検査から外している README（テンプレート例が frontmatter 検査で必ず偽陽性になる）も、
    # **リンクだけは見る**。規約の正本が唯一無検査という穴を残さない——見本のリンクは
    # コードフェンスの中にあるので除外済み。
    for d in TARGET_DIRS:
        for name in sorted(EXCLUDED_ENTIRELY):
            excluded = root / d / name
            if excluded.is_file():
                rel = excluded.relative_to(root).as_posix()
                check_body_links(
                    rel, root, excluded.read_text(encoding="utf-8"), errors, set(),
                    report_unclosed=True,
                )

    for extra in EXTRA_LINK_TARGETS:
        path = root / extra
        if path.is_file():
            check_body_links(
                extra, root, path.read_text(encoding="utf-8"), errors, set(), report_unclosed=True
            )

    actual_count: dict[str, int] = {cls: 0 for cls in declared}
    # REQ-ID → 初出パス。一意性はクラス内グローバル（文書をまたいで 1 つ）なので台帳は全体で 1 つ。
    req_seen: dict[str, str] = {}
    # 割当索引の突合用。キーは索引表と同じ `knowledge/x.md` 形式（docs/ を剥がす）。
    doc_class_by_rel: dict[str, list[str]] = {}
    scanned_rels: set[str] = set()
    # 全文書の sources の和集合。orphan ADR 検査（12）が「誰からも参照されていない ADR」を
    # 判定する材料。走査対象から外した README.md の sources は数えない——テンプレート例として
    # 存在しないパスを並べているので、数えると偽の被参照が生まれる。
    referenced_sources: set[str] = set()
    for path in targets:
        rel = path.relative_to(root).as_posix()
        scanned_rels.add(rel)
        text = path.read_text(encoding="utf-8")
        fm = parse_frontmatter(text)
        # frontmatter が無くてもリンクだけは見る（下の continue で丸ごと飛ばさない）。
        # fm が無い文書は REQ 走査（未閉じフェンスの報告元）まで到達しないので、
        # そのときだけこちらでフェンス閉じ忘れを報告する。
        link_seen: set[str] = set()
        check_body_links(rel, root, text, errors, link_seen, report_unclosed=not fm)
        if not fm:
            errors.append(f"{rel}: frontmatter が無い")
            continue

        # (1)(2)(3) doc_class / tags
        if path.name not in EXCLUDED_FROM_DOC_CLASS:
            classes = fm.get("doc_class")
            if not classes:
                errors.append(f"{rel}: doc_class が無い（書式は {REGISTRY} 参照）")
            else:
                if len(set(classes)) != len(classes):
                    errors.append(f"{rel}: doc_class に重複がある → {classes}")
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
                doc_class_by_rel[rel.removeprefix("docs/")] = classes

        # sources は REQ 表の出典突合（11）でも使うので、REQ 走査より先に読む。
        # **正規化層を挟まず、生の文字列のまま 4 / 6 / 11 / 12 の全部が同じ値を見る**
        # （非正規形は下の (4) が弾く。理由はそこに書いた）。
        sources = fm.get("sources", [])
        referenced_sources.update(sources)

        # (8)(11) REQ 表。リンクの台帳は上の本文検査と共有していて、本文で報告済みの
        # リンク先は REQ 側で二重に報告しない（REQ 表の行は本文にも含まれるため）。
        check_req_blocks(
            rel, text, fm.get("doc_class"), declared, req_seen, root, errors,
            set(sources), link_seen,
        )

        # (4) sources の実在
        if not sources:
            errors.append(f"{rel}: sources が空（由来を辿れない）")
        for src in sources:
            path_error = repo_relative_path_error(src)
            if path_error:
                errors.append(f"{rel}: sources は{path_error} → {src}")
            elif not (root / src).is_file():
                errors.append(f"{rel}: sources のパスが実在しない → {src}")
            elif not case_exact((root / src).resolve(), root.resolve()):
                # macOS(APFS) は大文字小文字を区別しないので is_file() が通り、Linux の CI
                # だけが落ちる。しかも手元では stale 判定が「履歴を辿れず」の warning に
                # 退化する（`./` 形式と同じ silent-green）。本文リンク側は既に見ている。
                errors.append(f"{rel}: sources の大文字小文字が実ファイルと違う → {src}")

        # (6) stale
        distilled = fm.get("distilled_from_sha", "")
        if not distilled:
            errors.append(f"{rel}: distilled_from_sha が無い")
            continue
        resolved = git("rev-parse", "--verify", "--quiet", f"{distilled}^{{commit}}")
        if resolved.returncode != 0:
            # shallow clone なら履歴が無いだけなので警告に留める。full clone で解決できない
            # のは frontmatter の誤りで、放置すると **その文書の stale 判定が丸ごと消える**
            # （fail-open）。CI は fetch-depth: 0 なので通常は error 側に来る。
            message = (
                f"{rel}: distilled_from_sha '{distilled}' を解決できない"
                "（この文書の stale 判定は行われない）"
            )
            (warnings if shallow else errors).append(
                message + "。shallow clone のため" if shallow else message
            )
            continue
        distilled_full = resolved.stdout.strip()
        for src in sources:
            if not (root / src).is_file():
                continue  # 実在チェックで既に error にしている
            changed = last_content_change(src)
            if isinstance(changed, ScanAborted):
                # 走査を完遂できなかった。**warning にしない**——これは環境の都合ではなく
                # 検査側の都合で、warning に落とすと「除外対象のコミットを積めば検査が消える」
                # fail-open が一段外側で再現する（この PR が塞いだのと同じ形）。
                errors.append(
                    f"{rel}: {src} の履歴走査を完遂できず stale 判定が行われていない"
                    f"（{changed.reason}）"
                )
                continue
            if changed is None:
                # 履歴が無い（未コミット・履歴の尽き・shallow）。黙って通すと fail-open に
                # なるので可視化する。
                warnings.append(
                    f"{rel}: {src} の履歴が無く stale 判定を実施できなかった"
                    "（未コミット / 履歴の尽き / shallow）"
                )
                continue
            if git("merge-base", "--is-ancestor", changed, distilled_full).returncode != 0:
                # #580 で warning → error に昇格。「ADR の内容は knowledge へ全部写す」
                # （ADR 0073 決定 2）の担保はこの検査だけで、warning のままだと写した量に
                # 比例して追従漏れが静かに溜まる。逃げ道は --warn-only のみ。
                # **この文言は scripts/bump-distilled-sha.py の --all-stale がパースする契約**
                # （`✗ <path>: STALE ← <src>`）。整えるときは向こうの RE_STALE_LINE も直す。
                errors.append(
                    f"{rel}: STALE ← {src} が distilled_from_sha({distilled}) より後に更新されている"
                    f"（{changed[:7]}）。差分マージして sha/日付を更新する"
                )

    # (12) orphan ADR。「ADR は必ずどこかの knowledge / specifications の sources から
    # 参照される」という不変条件を機械で守る（ADR 0083）。ここが空いていると、ADR を
    # 足した誰かが sources への登録を忘れただけで、その ADR は永久に watch 対象外になる。
    # glob は存在しないディレクトリでも例外を出さず空を返すので is_dir() ガードは要らない。
    # 非再帰なのでサブディレクトリの ADR は見えないが、そちらは check-adr-numbers.sh が
    # 階層 ADR を落とす担当（判定を二重に持たない）。
    adr_dir = root / ORIGINAL_DOCS
    adrs = sorted(
        p.relative_to(root).as_posix()
        for p in adr_dir.glob("*.md")
        if RE_ADR_FILENAME.match(p.name)
    )
    if not adrs:
        # ADR が 0 件になることはありえない。0 件＝判定条件かパスの取り違えなので、
        # 静かに緑にせず落とす（ここが無いと検査 12 が丸ごと無言で無効化される fail-open）。
        # **`--warn-only` でも抑止しない**——これは「違反がある」ではなく「検査が成立して
        # いない」側の失敗で、マーカー欠落と同じクラス。ただし sys.exit はしない
        # （それまでに溜めた errors / warnings を捨てるため。割当索引を先に読む理由と同じ）。
        errors.append(
            f"{ORIGINAL_DOCS}/ に ADR（0 埋め 4 桁）が 1 件も無い"
            "（判定条件かディレクトリを確認する。orphan 検査が丸ごと効いていない）"
        )
        fail_closed = True
    for adr in adrs:
        if adr in referenced_sources or adr in orphan_exceptions:
            continue
        errors.append(
            f"{adr}: どの knowledge / specifications の sources からも参照されていない"
            f"（ADR は必ずどこかへ写す。写せない ADR は {REGISTRY} の例外表に理由付きで登録する）"
        )
    # 例外表そのものの健全性。腐った例外を残すと、次に本物の orphan が出たときに
    # 「例外表にあるから安心」と誤読される（N/A 宣言表と一覧の相互突合と同じ理屈）。
    known_adrs = set(adrs)
    for adr in sorted(orphan_exceptions):
        if adr not in known_adrs:
            # 「ファイルが無い」と「ファイルはあるが ADR 名でない」は原因が別。混ぜると
            # issue 由来の一次資料（0 埋めしない番号）を登録したときに「実在しない」と
            # 出て、実在するファイルを探し回らせてしまう。
            if (root / adr).is_file():
                reason = (
                    "ADR ではない（0 埋め 4 桁のみが ADR。issue 由来の一次資料は"
                    "orphan 検査の対象外なので例外に挙げる必要が無い）"
                )
            else:
                reason = "ファイルが実在しない（パスは sources と同じリポジトリルート相対）"
            errors.append(f"{REGISTRY}: orphan 例外表の {adr} は{reason}")
        elif adr in referenced_sources:
            errors.append(
                f"{REGISTRY}: orphan 例外表の {adr} は実際には sources から参照されている"
                "（不要になった例外は消す）"
            )

    # (10) 割当索引と実ファイルの突合。上の (5) はクラス別の集計数しか見ないので、
    # 主クラスの順序入替や 2 文書間のクラス交換は素通りする（索引を突き合わせて塞ぐ）。
    for key in sorted(set(indexed) | set(doc_class_by_rel)):
        actual = doc_class_by_rel.get(key)
        listed = indexed.get(key)
        canonical = f"[{', '.join(actual)}]" if actual else None
        if listed is None:
            errors.append(f"{REGISTRY}: 割当索引に {key} の行が無い（実際は {canonical}）")
        elif actual is None:
            # 「ファイルが無い」と「ファイルはあるが doc_class を読めない」は原因が別。
            # 後者は本体側で別の error を出しているので、索引側で誤誘導しない。
            name = Path(key).name
            if name in EXCLUDED_FROM_DOC_CLASS:
                # doc_class を持たない設計の文書（規約・クラス定義そのもの）。
                # 索引は doc_class の一覧なので、そもそも行を置かない。
                reason = "doc_class を持たない文書なので索引に載せない"
            elif f"docs/{key}" not in scanned_rels:
                reason = "対応する検査対象の文書が無い"
            else:
                reason = "対応する文書の doc_class を読めない（上の error を先に直す）"
            errors.append(f"{REGISTRY}: 割当索引の {key} は{reason}")
        elif listed != canonical:
            errors.append(
                f"{REGISTRY}: 割当索引の {key} が frontmatter と一致しない"
                f"（索引={listed} / 実際={canonical}）"
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
        if warn_only and not fail_closed:
            print("  --warn-only のため 0 で終了する", file=sys.stderr)
            return 0
        if warn_only:
            print("  検査が成立していないため --warn-only でも 1 で終了する", file=sys.stderr)
        return 1

    print(
        f"✓ 文書クラス・sources 整合を確認"
        f"（{len(targets)} 本 / 実在を検査したリンク {LINK_COUNT} 本 / 警告 {len(warnings)} 件）"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
