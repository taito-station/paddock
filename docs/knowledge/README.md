# knowledge — 蒸留済み確定知の規約

dahatake/HypervelocityEngineering（HVE, MIT）の original-docs → qa → knowledge 蒸留モデルを
paddock に導入したもの。**蒸留は Claude Code が担う**（HVE 本体の LLM オーケストレータは持ち込まない）。

## 3 層モデル

```
docs/original-docs/  読み取り専用の一次資料（生素材）
        │  [Claude が読取・欠落/不整合を検出]
        ▼
docs/qa/             質問票 + 回答（人間 or Claude が回答）
        │  [Claude が差分マージ]
        ▼
docs/knowledge/ ＋ docs/specifications/   status 付き確定知（＝この層）
```

- **横断検索**は mdq（Markdown Query, BM25・ローカル）で全 docs を索引する。生ファイルを読む前に
  `scripts/mdq search` を使う（[.claude/skills/markdown-query/SKILL.md](../../.claude/skills/markdown-query/SKILL.md)）。
- **ADR（`docs/adr/`）は不変の決定記録**として据え置く。knowledge は決定の「なぜ」を frontmatter
  `sources` と本文リンクで ADR へ参照する（ADR は移動・改変しない）。

## knowledge はどこにあるか

- **`docs/specifications/`**: 既存のドメイン/機能知。**その場で knowledge に昇格**する（frontmatter を
  付与）。ADR が多数の履歴パス参照を持つため物理移動しない（リンク・決定記録を壊さないため）。
- **`docs/knowledge/`**: qa パイプライン由来の**新規・横断的な蒸留知**の置き場。既存 spec に属さない
  ものはここに置く。

どちらも下記 frontmatter 規約に従い、mdq の索引対象（`mdq.toml`）に含める。

## frontmatter 規約

```yaml
---
status: Confirmed        # Confirmed（確定）/ Tentative（暫定）/ Conflict（矛盾・要解消）
kind: knowledge
sources:                 # 由来（ADR / qa / original-docs のパス）。決定の「なぜ」を辿れるように
  - docs/adr/NNNN-....md
updated: "YYYY-MM-DD"    # 最終内容更新日（YAML の date 型を避けるため必ずクォート）
distilled_from_sha: "<short-sha>"  # この知が反映するリポジトリ状態の git SHA（トレーサビリティ）
---
```

> **注意**: `updated` は必ずダブルクォートで囲む。クォートしないと YAML が `date` 型に解釈し、mdq の
> 索引化（frontmatter を JSON 化）が `Object of type date is not JSON serializable` で失敗する。

- **status**: `Confirmed`=検証済みで運用の前提にしてよい / `Tentative`=検証中・暫定 /
  `Conflict`=source 間で矛盾があり要解消（放置しない）。
- **参照 SHA**: HVE `knowledge_versions.py`（参照 knowledge の git SHA を可視化）の軽量代替。
  由来ファイルやリポジトリ HEAD の SHA を `git log -1 --format=%h -- <path>` 等で記録し、
  「いつ時点の知か」を辿れるようにする。
- **変更履歴**: 本文末尾に `## 変更履歴` を置き、更新のたびに 1 行追記して `updated` を更新する。

## 昇格・更新の運用（Claude が回す蒸留）

1. 一次資料は `docs/original-docs/` に置く（RO・書き換えない）。
2. 調査で判明した Q&A は `docs/qa/` に質問票として起票し、回答を書き込む。
3. 回答済み qa と original-docs を突き合わせ、差分を knowledge に**差分マージ**（全書き換えしない・冪等）。
4. 矛盾は `status: Conflict` で明示し、解消してから `Confirmed` に上げる。
5. 決定を伴うものは ADR を別途起票し、knowledge の `sources` から参照する。
