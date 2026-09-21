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
  ├─ 重大な矛盾 → status: Conflict 宣言 → STOP
  └─ OK → Step 4

Step 4: カバレッジ分析
  └─ ギャップ報告 + QA 生成提案 → 完了
```

## 実行手順

### Step 1: Stale 検出

knowledge/ 配下の各ファイルについて、frontmatter の `distilled_from_sha` と source の現在の git sha を比較する。

1. knowledge/ 配下の全ファイルの frontmatter を読む
2. 各 `distilled_from_sha` エントリについて、source の現在の sha を取得:
   ```bash
   git log -1 --format=%H -- <source_file_path>
   ```
3. sha が一致しないファイルを stale として報告する

**出力**: stale なファイル一覧（ファイルパス、stale な source、sha の差分）

- stale なファイルが 0 件の場合: 「stale なし」と報告し、Step 3 に進む（Step 2 スキップ）
- stale なファイルがある場合: Step 2 に進む

### Step 2: 蒸留（差分マージ）

stale な各ファイルについて、source の変更を knowledge 本文に反映する。

1. source ファイルの変更差分を確認する:
   ```bash
   git diff <old_sha>..<new_sha> -- <source_file_path>
   ```
2. 変更内容を knowledge 本文に差分マージする
   - **全書き換え禁止**。変更箇所のみ更新する
   - source にない情報（既存の蒸留結果）は維持する
3. 決定を伴う変更がある場合は、決定ログ（`knowledge/adr/`）に新エントリを追加する
4. frontmatter の `distilled_from_sha` を新しい sha に更新する
5. frontmatter の `updated` を当日日付に更新する

**蒸留対象が 3 本以上の場合**: サブエージェントに委譲する（1 エージェント 1 ファイル）

### Step 3: 横断整合性レビュー

knowledge/ 配下の全文書を対象に、整合性を検査する。

#### 機械検査チェックリスト

- [ ] frontmatter の必須フィールド（title / status / kind / sources / distilled_from_sha / updated）が全ファイルに存在する
- [ ] `distilled_from_sha` で参照している source ファイルが実在する
- [ ] 決定ログの既存エントリが改変されていない（append-only 原則）
- [ ] status が Conflict のまま放置されている文書がない

#### 目視チェック項目

- [ ] 用語が文書間で統一されている
- [ ] status が文書の内容と整合している（Confirmed なのに TBD が残っている等）
- [ ] SoT 優先順位に違反していない（knowledge が source と矛盾していない）

#### 矛盾検出時の対応

| 重大度 | 対応 |
|---|---|
| 軽微（表記揺れ、フォーマット不備） | その場で修正 |
| 重大（事実の矛盾、SoT 違反） | 該当文書の status を `Conflict` に変更し、ユーザーに報告して STOP |

### Step 4: カバレッジ分析

doc_class（文書分類）が定義されている場合、分類ごとの充足状況を検査する。

1. knowledge/ 配下の全ファイルの `doc_class` を集計する
2. 分類ごとにファイル数と status の内訳を報告する
3. 以下を列挙する:
   - `Tentative` ステータスのファイル一覧
   - TBD が残っている箇所の一覧
   - 文書が存在しない分類（doc_class が定義されている場合）
4. 不足がある場合は QA 質問票の生成を提案する（ユーザー判断で実行）

**doc_class が未定義の場合**: 分類別集計をスキップし、status と TBD の集計のみ行う

## 完了報告

各実行完了時に以下を報告する:

1. Stale 検出結果（件数）
2. 蒸留したファイル一覧
3. 整合性レビュー結果（検出した問題と対応）
4. カバレッジ分析結果（ギャップ一覧）
5. 推奨する次のアクション
