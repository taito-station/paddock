# knowledge — 蒸留済み確定知の規約

dahatake/HypervelocityEngineering（HVE, MIT）の original-docs → qa → knowledge 蒸留モデルを
paddock に導入したもの。**蒸留は Claude Code が担う**（HVE 本体の LLM オーケストレータは持ち込まない）。

## 3 層モデル

```
docs/original-docs/  読み取り専用の一次資料（生素材 + ADR）
        │  [Claude が読取・欠落/不整合を検出]
        ▼
docs/qa/             質問票 + 回答（人間 or Claude が回答）
        │  [Claude が差分マージ]
        ▼
docs/knowledge/ ＋ docs/specifications/   status 付き確定知（＝この層。読むのはここ）
```

- **横断検索**は mdq（Markdown Query, BM25・ローカル）で全 docs を索引する。生ファイルを読む前に
  `scripts/mdq search` を使う（[.claude/skills/markdown-query/SKILL.md](../../.claude/skills/markdown-query/SKILL.md)）。
- **ADR は一次資料層（`docs/original-docs/`）に属する不変の決定記録**（ADR 0073 で旧 `docs/adr/` から
  統合）。ADR も一次資料も「書き換えない（RO）」という性質が同じで、`ADR → knowledge` の写しを
  規約どおりの蒸留として扱えるため。**一度置いた ADR は改変しない**（決定を変えるときは新しい ADR で
  supersede する）。
- **確定知を読む入口は knowledge**。ADR の決定・理由・却下案・影響は knowledge に**全部写す**。
  重複を許す代わりに、同期切れ（`sources` が更新されたのに蒸留が追従していない状態）は
  **機械検査で検出する**——写した量に比例して stale 面積が増えるため、人手の規律には委ねない。
- **`docs/original-docs/` の命名は 2 系統**（`check-adr-numbers.sh` の判定根拠。
  詳細は [docs/original-docs/README.md](../original-docs/README.md)）:
  - ADR = **0 埋め 4 桁**（`0001-`〜`0999-`）
  - issue 由来の一次資料 = **GitHub issue 番号（0 埋めしない）**（`382-`, `401-` …）

## knowledge はどこにあるか

- **`docs/specifications/`**: 既存のドメイン/機能知。**その場で knowledge に昇格**する（frontmatter を
  付与）。多数の相互リンクを持つため物理移動しない。
- **`docs/knowledge/`**: qa パイプライン由来の**新規・横断的な蒸留知**の置き場。既存 spec に属さない
  ものはここに置く。

どちらも下記 frontmatter 規約に従い、mdq の索引対象（`mdq.toml`）に含める。

## frontmatter 規約

```yaml
---
status: Confirmed        # Confirmed（確定）/ Tentative（暫定）/ Conflict（矛盾・要解消）
kind: knowledge
sources:                 # 由来（ADR / qa / original-docs のパス）。決定の「なぜ」を辿れるように
  - docs/original-docs/0NNN-....md   # ADR は 0 埋め 4 桁
  - docs/qa/QA-....md
distilled_from_sha: "<short-sha>"  # この知が反映するリポジトリ状態の git SHA（トレーサビリティ）
updated: "YYYY-MM-DD"    # 内容を実質更新した日（YAML の date 型を避けるため必ずクォート。詳細な履歴は git log を正とする）
---
```

> **注意**: `updated` は必ずダブルクォートで囲む。クォートしないと YAML が `date` 型に解釈し、mdq の
> 索引化（frontmatter を JSON 化）が `Object of type date is not JSON serializable` で失敗する。

- **status**: `Confirmed`=検証済みで運用の前提にしてよい / `Tentative`=検証中・暫定 /
  `Conflict`=source 間で矛盾があり要解消（放置しない）。
- **参照 SHA**: HVE `knowledge_versions.py`（参照 knowledge の git SHA を可視化）の軽量代替。
  原則は**この知を蒸留した時点のリポジトリ HEAD** を `git rev-parse --short HEAD` で記録する
  （pilot の `probability-estimation.md` もこの方式）。特定の由来ファイル版に紐付けたいときは
  `git log -1 --format=%h -- <path>` を使う。いずれも「いつ時点の知か」を辿れるようにするのが目的。
- **変更履歴**: **git log を正とする**（変更の追跡は履歴で辿る）。内容を実質更新したら `updated` と
  `distilled_from_sha` を更新すれば足りる。本文末尾の `## 変更履歴` セクションは**任意**——
  節目や意図を人間可読に残したいときだけ置く（一括後付けはしない）。既に `## 変更履歴` を持つ 2 本
  （[`docs/specifications/probability-estimation.md`](../specifications/probability-estimation.md) /
  [`docs/knowledge/analyze-search-and-state.md`](analyze-search-and-state.md)）はそのまま維持してよい。

## 昇格・更新の運用（Claude が回す蒸留）

1. 一次資料は `docs/original-docs/` に置く（RO・書き換えない）。
2. 調査で判明した Q&A は `docs/qa/` に質問票として起票し、回答を書き込む。
3. 回答済み qa と original-docs を突き合わせ、差分を knowledge に**差分マージ**（全書き換えしない・冪等）。
4. 矛盾は `status: Conflict` で明示し、解消してから `Confirmed` に上げる。
5. 決定を伴うものは ADR を `docs/original-docs/0NNN-*.md` に起票し（採番は
   `scripts/check-adr-numbers.sh next`）、knowledge の `sources` から参照する。**ADR の決定・理由・
   却下案・影響は knowledge へ全部写す**（読む入口を knowledge に一本化するため）。
6. **sources 追従**: knowledge の `sources` に列挙されたファイルを変更する PR は、参照元 knowledge の
   `distilled_from_sha` と `updated` を現 HEAD に更新する。本文が変わる場合は差分マージを行い、
   変わらない場合は sha と日付の bump のみで足りる。**この追従は機械検査の対象**（実例: 
   [`app-bootstrap.md`](app-bootstrap.md) が `status: Confirmed` のまま、qa 側で「#453 で覆る」と
   追記済みの `NoopParser` を推奨し続けた事故がある。人手の規律だけでは守れない）。
