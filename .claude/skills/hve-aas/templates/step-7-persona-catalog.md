# Step 7: ペルソナカタログ

## 目的

ユースケース文書から横断的なペルソナ（アクター・ロール）を抽出する。
APP-ID 横断の SoT として、複数 APP-ID に登場する同一ペルソナの重複定義を防ぐ。

## 入力

- `docs/catalog/use-case-catalog.md`（必須）
- `docs/catalog/app-catalog.md`（必須）

## 出力

- `docs/catalog/persona-catalog.md`

## 抽出ルール

- ユースケースのアクター記述を一次ソースとする
- 同一の業務的役割を持つアクターが複数 APP-ID に登場する場合は 1 ペルソナに統合する
- **人間のペルソナのみ**（システムアクターは含めない）

## ID 採番

- `PRS-XXX` 形式（PRS = Persona）
- ユースケースに既存アクター ID がある場合は流用する

## 出力フォーマット

```markdown
# ペルソナカタログ

| persona_id | persona_name | category | description | source_actor_names | referenced_ucs | linked_apps |
|---|---|---|---|---|---|---|
| PRS-001 | [名称] | [分類] | [説明] | [元のアクター名] | UC-001, UC-003 | APP-001, APP-002 |
```

### 要確認候補

業務上必要性が示唆されるが根拠不足のペルソナ:

| persona_name | 示唆元 | 不足している根拠 |
|---|---|---|

## 制約

- 各ペルソナに最低 1 件の `referenced_ucs` が必須（`use-case-catalog.md` に実在すること）
- `referenced_ucs` への TBD は禁止
- 根拠のないペルソナは正式登録しない（「要確認候補」表に分離する）
- 捏造禁止 / オーバーエンジニアリング禁止
