---
name: akm
description: >
  Autonomous Knowledge Management（HVE 準拠）。knowledge/specifications の stale 検出・
  蒸留・横断整合性レビュー・カバレッジ分析を 4 ステップで実行する。
  USE FOR: knowledge の定期メンテナンス、stale 解消、蒸留サイクルの完走確認。
  DO NOT USE FOR: 個別ファイルの編集（直接 Edit する）、実装作業（issue スキル等を使う）。
  WHEN: 「/akm」の直接呼び出し、「knowledge 更新して」「蒸留して」「stale 確認」等の発言。
metadata:
  origin: user
  version: "1.0.0"
---

# akm — Autonomous Knowledge Management

HVE（HypervelocityEngineering）の AKM ワークフローを Claude Code 向けに実装したスキル。
`docs/knowledge/` と `docs/specifications/` の鮮度・整合性を維持する。

規約の正本は [docs/knowledge/README.md](../../../docs/knowledge/README.md)。
蒸留ルールの詳細は [references/distillation-guide.md](references/distillation-guide.md)。

---

## 手順

### Step 1: Stale 検出

stale な knowledge/specifications を特定する。

```sh
python3 scripts/bump-distilled-sha.py --all-stale --dry-run
```

- **「STALE な文書は無い」** → Step 3 へスキップ（蒸留不要、整合性レビューのみ）
- **stale が 1 件以上** → 出力から「stale なファイル」と「変更された source」のペアを読み取り、Step 2 へ

stale の出力をユーザーに報告する:

```
📋 Stale 検出結果:
- docs/knowledge/xxx.md ← source docs/docs-original/NNN-yyy.md が変更
- docs/specifications/zzz.md ← source docs/qa/QA-www.md が変更
```

---

### Step 2: 蒸留（差分マージ）

stale な各 knowledge/specifications に対して差分マージを実行する。

**蒸留対象が 3 本以上の場合**: サブエージェント（`impl-sonnet`）に 1 ファイルずつ委譲する。
**2 本以下の場合**: メインループで直接実行する。

各ファイルの蒸留手順:

1. **変更された source を読む**（docs-original / qa）
2. **現行の knowledge/specifications を読む**
3. **差分を特定する**: source の変更のうち、knowledge の本文に反映すべき箇所を洗い出す
4. **差分マージを実行する**: 全書き換えしない。変更箇所のみ更新する
   - 新しい事実 → 本文の該当セクションに追記
   - 変更された事実 → 本文の該当箇所を更新
   - 廃止された事実 → 本文から削除または `Retired` に更新
5. **frontmatter を更新する**:
   - `distilled_from_sha`: `scripts/bump-distilled-sha.py <file>` で更新
   - `updated`: 本文が実質変わった場合のみ日付を更新（sha だけの bump では触らない）
6. **決定を伴う変更があれば決定ログに append する**

蒸留の詳細ルール（REQ テーブルの扱い・paddock 固有 D クラスの注意点）は
[references/distillation-guide.md](references/distillation-guide.md) を参照。

蒸留が完了したら検証する:

```sh
python3 scripts/check-doc-classes.py
python3 scripts/bump-distilled-sha.py --all-stale --dry-run   # 0 件であること
```

---

### Step 3: 横断整合性レビュー

蒸留の有無にかかわらず実行する。チェック項目は
[references/consistency-checklist.md](references/consistency-checklist.md) が正本。

1. **機械検査を実行する**:

```sh
python3 scripts/check-doc-classes.py
python3 scripts/check-decision-log-immutability.py
```

2. **機械検査が届かない範囲を目視で確認する**:
   - glossary.md（D07）の定義と各文書の用語使用が一致しているか
   - Confirmed な knowledge が Tentative な source を根拠にしていないか
   - CLAUDE.md の買い方ルール等と specifications の記述が矛盾していないか
   - 決定ログで supersede されたエントリの本文側が更新されているか

3. **矛盾を検出したら**:
   - 軽微（用語の表記ゆれ等）→ その場で修正
   - 重大（事実の矛盾・SoT 違反）→ `status: Conflict` を宣言し、ユーザーに報告して判断を仰ぐ

---

### Step 4: カバレッジ分析

knowledge 層の充足度を分析し、不足を報告する。

1. **D クラスの充足ギャップを確認する**:

```sh
python3 scripts/check-doc-classes.py --warn-only 2>&1 | grep '充足ギャップ'
```

2. **既存 knowledge の Tentative / Unknown を列挙する**:
   - frontmatter が `status: Tentative` のファイルを抽出
   - 本文中の `TBD` / `Unknown` / `未定` をリストアップ

3. **報告をまとめる**:

```
📊 カバレッジ分析:
■ Stale: 0 件（Step 1-2 で解消済み）
■ 機械検査: OK / NG（詳細）
■ 充足ギャップ: D05, D16, D18（active だが文書 0 本）
■ Tentative 文書: N 本（リスト）
■ TBD 項目: N 件（ファイル:行）
■ 提案: 以下の QA を生成すると充足度が上がる
  - D05: ...
  - ...
```

4. **QA 生成を提案する**（実行はユーザー判断）:
   - カバレッジが低い D クラスに対して、生成すべき QA のテーマを提案する
   - ユーザーが承認したら `docs/qa/QA-{topic}-{issue}.md` を生成する
   - 生成した QA に回答を書き込み、Step 2 の蒸留に戻る（反復精緻化ループ）

---

## 注意事項

- **docs-original/ は読み取り専用**。このスキルで変更するのは knowledge/specifications/qa のみ
- **全書き換えしない**。差分マージが基本。既存の知識を壊さない
- **SoT 優先順位**: docs-original > qa（Confirmed 回答）> knowledge。矛盾時は上位が勝つ
- **決定ログは append-only**。既存エントリを書き換えない
