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
  8. REQ 表（要件 ID）の書式・一意性・status・Confirmed の検証手段            [error]

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

import functools
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


def is_metadata_only_change(sha: str, path: str) -> bool:
    """そのコミットの変更が frontmatter のメタデータだけかを判定する。

    ADR 0073 の移設は sources のパス表記を一斉に書き換えた。本文は 1 文字も変わって
    いないのに「内容変更」と見なすと、それを sources に持つ knowledge が軒並み stale に
    なる（実測 7 件）。文書クラスの付与も同じ形で自己ノイズを生む。
    """
    new = git("show", f"{sha}:{path}")
    old = git("show", f"{sha}^:{path}")
    if new.returncode != 0 or old.returncode != 0:
        return False  # 初回追加や親を辿れない場合は判定しない（内容変更として扱う）
    new_fm, new_body = split_frontmatter(new.stdout)
    old_fm, old_body = split_frontmatter(old.stdout)
    if new_fm is None or old_fm is None or new_body != old_body:
        return False
    new_blocks, old_blocks = frontmatter_blocks(new_fm), frontmatter_blocks(old_fm)
    changed = {k for k in set(new_blocks) | set(old_blocks) if new_blocks.get(k) != old_blocks.get(k)}
    return bool(changed) and changed <= METADATA_KEYS


def path_status(sha: str, path: str) -> "tuple[str | None, str | None]":
    """コミット sha における path の (status, リネーム元) を返す。

    name-status に **パス指定を渡さない**。渡すとリネーム元が絞り込みから外れて対にならず、
    R100 ではなく A（新規追加）として報告される（実測）。コミット全体の name-status を取り、
    対象パスを終点とする行だけを見る。
    core.quotePath=false を明示するのは、既定だと非 ASCII パスが "\\346\\234\\200..." の
    クォート表記になり終点一致が外れて R100 除外が破れるため。
    """
    shown = git(
        "-c", "core.quotePath=false", "show", "--format=", "--name-status", "-M100%", sha
    ).stdout
    for line in shown.splitlines():
        parts = line.split("\t")
        if len(parts) >= 2 and parts[-1] == path:
            status = parts[0]
            src = parts[1] if status.startswith("R") and len(parts) >= 3 else None
            return status, src
    return None, None


@functools.lru_cache(maxsize=None)
def last_content_change(path: str, limit: int = 40, max_renames: int = 10) -> "str | None":
    """path の**内容**が最後に変わったコミットの SHA。

    次の 2 種類は「内容変更ではない」として遡る:
      - R100（内容差分ゼロのリネーム）。ディレクトリ移設で全件が stale になるのを防ぐ
      - frontmatter のメタデータだけの変更（sources のパス追従・doc_class 付与など）

    **`--follow` は使わない。** `--follow` はリネームで履歴を打ち切ることがあり（実測:
    ADR 0036 は移設コミット 1 件しか返さず、それ以前の起票コミットへ辿れなかった）、
    そこで打ち切られると「履歴を辿れない＝判定不能」に落ちる。代わりに、R100 を見つけたら
    **そのコミットの親からリネーム元のパスで履歴を取り直す**。リネームが何段重なっても効く。
    """
    current = path
    tip = "HEAD"
    for _ in range(max_renames):
        proc = git("log", f"--max-count={limit}", "--format=%H", tip, "--", current)
        if proc.returncode != 0:
            return None
        shas = proc.stdout.split()
        if not shas:
            return None
        renamed = False
        for sha in shas:
            status, rename_src = path_status(sha, current)
            if status is None:
                # そのコミットは current を（この名前では）触っていない。マージコミットは
                # git show が既定で差分を出さないためここに来る。実際の変更は親側のコミットに
                # 現れ、それも log の対象なので飛ばして問題ない。
                continue
            if status == "R100":
                if not rename_src:
                    return sha  # リネーム元を取れない＝判定できないので内容変更扱い
                current = rename_src
                tip = f"{sha}^"
                renamed = True
                break
            if is_metadata_only_change(sha, current):
                continue
            return sha
        if not renamed:
            return None  # この系列は全部「内容変更ではない」だった
    return None


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
RE_REQ_ID = re.compile(r"^REQ-(D\d{2})-(\d{3})$")
# タイトル付き `[x](path "題")` と山括弧付き `[x](<path>)` も拾う。素朴な
# `\(([^)\s]+)\)` だとタイトルを付けるだけで実在検査・絶対パス拒否を全部迂回できる。
RE_MD_LINK = re.compile(r"""\[[^\]]*\]\(\s*<?([^)\s>]+)>?(?:\s+[^)]*)?\)""")
# インラインコード。検証手段の列にはコマンドを書くので、その中のリンク様文字列
# （`grep '[x](nope.md)' file` 等）を実リンクとして検査すると偽陽性になる。
RE_INLINE_CODE = re.compile(r"`[^`]*`")
RE_TABLE_SEP_CELL = re.compile(r"^:?-+:?$")

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
    # (フェンス文字, 長さ)。同種かつ**同長以上**でのみ閉じる（GFM）。長さを見ないと、
    # ```` で囲んで ``` の見本を書く定番の書き方で内側が誤って閉じ、見本が実データに化ける。
    fence: "tuple[str, int] | None" = None
    for lineno, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        opener = RE_FENCE.match(stripped)
        if opener:
            token = opener.group(1)
            if fence is None:
                fence = (token[0], len(token))
            elif token[0] == fence[0] and len(token) >= fence[1]:
                fence = None
            continue
        if fence is not None:
            continue

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
            elif RE_REQ_ID_ANY.search(stripped):
                # GFM は行頭パイプを省いても表になる。黙って捨てると、その要件が
                # 一意性・status・空セルの検査から丸ごと消える（重複が通る）。
                errors.append(
                    f"{lineno} 行目: REQ ブロック内に表の行として読めない REQ 行がある"
                    f" → {stripped}（行頭を `|` で始める）"
                )
        elif RE_REQ_HEADER.match(stripped) or (
            "|" in stripped and RE_REQ_ID_ANY.search(stripped)
        ):
            errors.append(
                f"{lineno} 行目: REQ 表がマーカーの外にある → {stripped}"
                "（マーカーで囲まないと一意性・status の検査から丸ごと漏れる）"
            )

    if fence is not None:
        errors.append("コードフェンスが閉じられていない（以降の REQ 表が無検査になる）")
    if cls is not None:
        errors.append(f"REQ ブロック（{cls}）が閉じられていない")
        blocks.append((cls, rows))
    return blocks, errors


def check_req_blocks(
    rel: str,
    text: str,
    doc_classes: "list[str] | None",
    declared: dict,
    seen: "dict[str, str]",
    root: Path,
    errors: list[str],
) -> None:
    """1 文書ぶんの REQ 表を検査する。`seen` は REQ-ID → 初出パスの全体台帳。"""
    blocks, structural = parse_req_blocks(text)
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
            # 同じ理由——由来を辿れないなら要件の根拠を確認できない。
            linked = RE_INLINE_CODE.sub(" ", origin) + " " + RE_INLINE_CODE.sub(" ", verification)
            for target in RE_MD_LINK.findall(linked):
                if target.startswith(("http://", "https://", "mailto:", "#")):
                    continue
                cleaned = target.split("#", 1)[0]
                # sources 検査（下）と同じ理由でリポジトリ外を弾く。絶対パスを許すと
                # Path 連結が左辺を捨てて外を指し、「由来はリポジトリ内で辿れる」という
                # 前提が崩れる（`[外](/etc/hosts)` が実在扱いで通る）。
                if cleaned.startswith("/"):
                    errors.append(
                        f"{rel}: {req_id} のリンクは文書からの相対パスで書く → {target}"
                    )
                    continue
                resolved = ((root / rel).parent / cleaned).resolve()
                if not resolved.is_relative_to(root.resolve()):
                    errors.append(
                        f"{rel}: {req_id} のリンクがリポジトリ外を指している → {target}"
                    )
                elif not resolved.is_file():
                    errors.append(f"{rel}: {req_id} のリンク先が実在しない → {target}")


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

    # --- 対象ファイルを走査 ---
    shallow = is_shallow()
    targets: list[Path] = []
    for d in TARGET_DIRS:
        targets.extend(sorted(p for p in (root / d).glob("*.md") if p.name not in EXCLUDED_ENTIRELY))
        # glob は非再帰。サブディレクトリに .md を置かれると無検査域になるので可視化する
        # （現状 docs/specifications/diagrams/ に .md は無い）。
        nested = sorted(p.relative_to(root).as_posix() for p in (root / d).glob("*/**/*.md"))
        for n in nested:
            warnings.append(f"{n}: サブディレクトリの .md は検査対象外（直下に置く）")

    actual_count: dict[str, int] = {cls: 0 for cls in declared}
    # REQ-ID → 初出パス。一意性はクラス内グローバル（文書をまたいで 1 つ）なので台帳は全体で 1 つ。
    req_seen: dict[str, str] = {}
    for path in targets:
        rel = path.relative_to(root).as_posix()
        text = path.read_text(encoding="utf-8")
        fm = parse_frontmatter(text)
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

        # (8) REQ 表
        check_req_blocks(rel, text, fm.get("doc_class"), declared, req_seen, root, errors)

        # (4) sources の実在
        sources = fm.get("sources", [])
        if not sources:
            errors.append(f"{rel}: sources が空（由来を辿れない）")
        for src in sources:
            # 絶対パスや .. を許すと Path(root) / "/etc/hosts" が root を捨てて外を指し、
            # 「由来はリポジトリ内で辿れる」という検査の前提が崩れる。
            if src.startswith("/") or ".." in Path(src).parts:
                errors.append(f"{rel}: sources はリポジトリ相対パスで書く → {src}")
            elif not (root / src).is_file():
                errors.append(f"{rel}: sources のパスが実在しない → {src}")

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
            if changed is None:
                # 履歴を辿れなかった（未コミット・打ち切り等）。黙って通すと fail-open に
                # なるので可視化する。
                warnings.append(f"{rel}: {src} の履歴を辿れず stale 判定を実施できなかった")
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
