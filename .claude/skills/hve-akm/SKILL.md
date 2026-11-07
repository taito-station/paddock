---
name: hve-akm
description: |
  Automated Knowledge Management — knowledge/ のメンテナンスパイプライン。
  Stale 検出・蒸留・横断整合性レビュー・カバレッジ分析の 4 段階で知識を最新に保つ。
  「AKM」「ナレッジ更新」「knowledge メンテ」で起動。
---

# Automated Knowledge Management（AKM）

knowledge/ 配下の文書を source（docs-original/ および qa/）と同期し、
整合性とカバレッジを維持するメンテナンスパイプライン。

## 前提条件

- `rules/hve/knowledge-maturity.md` の frontmatter 標準に従った knowledge 文書が存在すること
- source ファイル（`docs-original/` および `qa/`）が git 管理されていること

## SoT 優先順位

`rules/hve/knowledge-maturity.md` の「SoT（Single Source of Truth）優先順位」に従う。

## パイプライン全体像

```
Step 1: Stale 検出
  └─ stale なファイル一覧 → Step 2

Step 2: 蒸留（差分マージ）
  └─ knowledge 本文を更新 → Step 3

Step 3: 横断整合性レビュー
  ├─ 軽微な矛盾 → その場修正
  ├─ 重大な矛盾（既存の Conflict 文書を除く） → status: Conflict 宣言 → STOP
  └─ OK → Step 4

Step 4: カバレッジ分析
  └─ ギャップ報告 + QA 生成提案 → 完了
```

## 実行手順

Step 2・Step 3 で knowledge を書き換える前に（サブエージェントに委譲するときは委譲の前にメインループが）、`main` / `master` 上にいれば作業ブランチを切る。名前は導入先のブランチ運用規約に従い、無ければ `docs/akm-<YYYYMMDD>`（同名のブランチがあれば末尾に `-2` などの連番を付ける）。

### Step 1: Stale 検出

検査スクリプトで knowledge/ 配下の文書を検査する（判定ロジックは `rules/hve/knowledge-maturity.md` の「Stale 検出の仕組み」）:

```bash
python3 .claude/scripts/hve/check-knowledge.py
```

導入先が hve-local（`.claude/rules/hve-local/hve-akm.md` など）で引数（`--required`・`--allow-empty-sources-with-decision-log` など）を定めていれば、それを付ける。

**出力**: `STALE` 行（文書・stale な source・`distilled_from_sha`）、その他の違反（必須項目の欠落・sources の不在など）、解消待ち（status が `Conflict`）の件数

- stale なファイルが 0 件の場合: 「stale なし」と報告し、Step 3 に進む（Step 2 スキップ）
- stale なファイルがある場合: Step 2 に進む
- exit 2（判定不能。shallow clone など）の場合: 原因を報告して STOP

### Step 2: 蒸留（差分マージ）

stale な各ファイルについて、source の変更を knowledge 本文に反映する。status が `Conflict` の文書は蒸留せず、解消待ちとして報告する（宣言後に source が変わると Step 1 で stale と出続けるが、それで正しい）。

1. source ファイルの変更差分を確認する:
   ```bash
   git diff <distilled_from_sha>..HEAD -- <source_file_path>
   ```
2. 変更内容を knowledge 本文に差分マージする
   - **全書き換え禁止**。変更箇所のみ更新する
   - source にない情報（既存の蒸留結果）は維持する
3. 決定を伴う変更がある場合は、決定ログ（既定は `knowledge/adr/`。インライン方式を宣言したプロジェクトは各 knowledge の `## 決定ログ` 節）に新エントリを追加する
4. 本文の変更をコミットしたあと、`python3 .claude/scripts/hve/bump-distilled-sha.py <文書>` で `distilled_from_sha` をそのコミットへ進め、追従のコミットを積む（コミットは自分の sha を含められないので 2 コミットになる。squash・rebase の後の扱いは knowledge-maturity の「sha の追従」）
5. frontmatter の `updated` は、本文または status が実質的に変わった場合だけ当日日付に更新する（`distilled_from_sha` だけの追従では触らない。判断は `rules/hve/knowledge-maturity.md` の「`updated` の規則」に従う）

**蒸留対象が 3 本以上の場合**: サブエージェントに委譲する（1 エージェント 1 ファイル）

### Step 3: 横断整合性レビュー

knowledge/ 配下の全文書を対象に、整合性を検査する。

#### 機械検査チェックリスト

- [ ] `python3 .claude/scripts/hve/check-knowledge.py` が exit 0（frontmatter の必須フィールド、sources の実在、stale）
- [ ] `python3 .claude/scripts/hve/check-decision-log.py` が exit 0（決定ログの既存エントリに、artifact-management の「書き換えてよい範囲」以外の改変がない。append-only 原則）。警告が出たら、誤字修正かどうかを確かめる
- [ ] status が Conflict の文書を解消待ちとして列挙し、報告に含めている（`check-knowledge.py` の解消待ちの一覧）

#### 目視チェック項目

- [ ] 用語が文書間で統一されている
- [ ] status が文書の内容と整合している（Confirmed なのに TBD が残っている等）
- [ ] SoT 優先順位に違反していない（knowledge が source と矛盾していない）

#### 矛盾検出時の対応

| 重大度 | 対応 |
|---|---|
| 軽微（表記揺れ、フォーマット不備） | その場で修正 |
| 重大（事実の矛盾、SoT 違反） | 該当文書の status を `Conflict` に変更し、ユーザーに報告して STOP。`distilled_from_sha` と `updated` の扱いは `rules/hve/knowledge-maturity.md` の「status の 3 段階」に従う（※）。すでに status が `Conflict` の文書は対象外（宣言し直さず、機械検査の解消待ち一覧に載せるだけ）。STOP するのは、この回に新しく Conflict を宣言したときだけ |

※ Step 2 で差分マージしていたら、本文と sha をどちらも蒸留前に戻す。その差分のために手順 3 で足した、まだコミットしていない決定ログのエントリも取り消す

### Step 4: カバレッジ分析

doc_class（文書分類）が定義されている場合、分類ごとの充足状況を検査する。

1. 集計はコマンドで取る。対象は check-knowledge.py と同じ（`knowledge/adr/` と `knowledge/README.md` は対象外）。frontmatter の `doc_class`・`status` と、TBD が残っている行番号を、文書ごとに 1 行（タブ区切り）で出す:
   ```bash
   python3 - <<'PY'
   import pathlib, re
   root = pathlib.Path("knowledge")
   for p in sorted(root.rglob("*.md")):
       rel = p.relative_to(root).as_posix()
       if rel.startswith("adr/") or rel == "README.md":
           continue
       lines = p.read_text(encoding="utf-8").splitlines()
       meta = {}
       if lines and lines[0].strip() == "---":
           for line in lines[1:]:
               if line.strip() == "---":
                   break
               m = re.match(r"(doc_class|status):\s*(.*?)\s*(?:#.*)?$", line)
               if m:
                   meta[m.group(1)] = m.group(2).strip("\"'")
       tbd = [str(i + 1) for i, line in enumerate(lines) if "TBD" in line]
       print("\t".join([meta.get("doc_class", ""), meta.get("status", ""), str(p), "TBD:" + ",".join(tbd)]))
   PY
   ```
2. 出力から、分類ごとのファイル数と status の内訳、`Tentative` のファイル一覧、TBD が残っている箇所、文書が存在しない分類（doc_class が定義されている場合）を報告する
3. 不足がある場合は QA 質問票の生成を提案する（ユーザー判断で実行）

**doc_class が未定義の場合**: 分類別集計をスキップし、status と TBD の集計のみ行う

## 完了報告

各実行完了時に、output-quality ルールの 6 セクション報告（目的 / 変更点 / 影響範囲 / 検証結果 / 既知の制約 / 次のステップ）を行い、以下をその中に含める（1・2 は変更点、3・4 は検証結果と既知の制約、5 は次のステップ）:

1. Stale 検出結果（件数）
2. 蒸留したファイル一覧（作業ブランチを切ったときはそのブランチ名も）
3. 整合性レビュー結果（検出した問題と対応）
4. カバレッジ分析結果（ギャップ一覧）
5. 推奨する次のアクション（作業ブランチを切ったときは PR の作成。マージは merge commit で行い、squash・rebase したときは knowledge-maturity の「sha の追従」に従う）
