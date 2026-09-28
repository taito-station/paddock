# Step 4.2: 個別 APP 要件定義

## このステップの目的

`docs/catalog/app-catalog.md` に列挙された全 APP について、それぞれの要求定義書を **単一セッション内で
出現順に** upsert する。**fan-out はしない**（並列書込みによる既存 confirmed 内容の上書き事故を避けるため）。

## 入力

- `docs/catalog/app-catalog.md`
- `docs/catalog/use-case-catalog.md`
- 既存の `docs/architectural-requirements-app-NNN.md`（存在する場合。既存内容は保持して upsert する）

## 出力

- `docs/architectural-requirements-app-NNN.md`（app-catalog に列挙された全 APP 分）

## 出力契約（固定 schema）

各 APP の要求定義書は以下を正確に持つ。

```markdown
Schema-Version: 1
APP-ID: APP-NNN
APP名: <app-catalog の名称>
Document-Status: active

## Requirements

| Requirement ID | Status | Requirement | Source | Acceptance Criteria | Blocker |
|---|---|---|---|---|---|
```

- Requirement ID は `APP-NNN-FR-NNN` / `APP-NNN-NFR-NNN` / `APP-NNN-C-NNN` のいずれか。kind ごとに
  `001`〜`999` を採番する。
- Status は `confirmed` / `source-backed` / `TBD` のいずれか。Blocker は `yes` / `no` のいずれか。

## 根拠の優先順位

既存 confirmed > 明示添付資料・回答済み QA > ARD 成果物（use-case-catalog 等） > 推論による `TBD`

- 上位根拠と競合する場合は上書きせず、Blocker として停止する。
- Source は実在する文書・節・回答を記録する。根拠が無い場合は `TBD` とする。
- `TBD` を確定要件として書かない。

## upsert 規則

- 既存 `confirmed` 行の ID と内容を変更しない。
- 既存 `source-backed` 行の ID を変更しない。
- 人手追記を削除しない。既存 ID を再番号付けしない。
- 新規 ID は同じ APP・kind の最大番号の次を割り当てる。999 を超える場合は停止する。
- `app-catalog.md` から消えた APP の文書（orphan）は削除せず、完了報告に警告として列挙する。

## 実行手順

1. `docs/catalog/app-catalog.md` から APP-ID と APP 名を出現順に抽出する。
2. 1 APP ずつ順番に処理する（fan-out しない）。
3. 既存文書があれば検証してから upsert し、なければ schema に従って新規作成する。
4. 全 APP の文書が存在し、schema を満たすことを確認する。
5. orphan 文書は保持し、警告として記録する。

## 完了報告に含める内容

- 処理 APP 数、作成数、更新数
- 競合により Blocker 化した件数
- orphan 文書数

## 重要な注意事項

- APP 間の並列書込みをしない（単一エージェントで順次処理する）。
- 既存 confirmed 内容・source-backed ID・人手追記・orphan 文書を削除しない。
- **捏造禁止**: ID・要件・数値・Source を根拠なく生成しない。不明は `TBD` と明示する。
- **オーバーエンジニアリング禁止**: 要求表以外の非決定的な独自 schema を追加しない。指示・要件にない
  汎用化・抽象化・将来拡張の先回りを行わない。YAGNI に従う。
