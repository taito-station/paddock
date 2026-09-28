# Step 4: データカタログ

## 目的

データモデルとドメイン分析を根拠に、概念エンティティ × 物理テーブル/列のマッピングを記録するデータカタログを作成する。

## 入力

- `docs/catalog/data-model.md`（必須）
- `docs/catalog/domain-analytics.md`（必須）
- `docs/catalog/app-catalog.md`（必須）
- `docs/catalog/service-catalog.md`（存在すれば参照）

## 出力

- `docs/catalog/data-catalog.md`

## 実行手順

1. データモデルのエンティティカタログから Entity-Table Mapping を作成する
2. 各エンティティの列定義を物理テーブルの列にマッピングする
3. Ownership Matrix: 各エンティティを読み書きするサービスを記載する
4. 各エンティティに「利用 APP」（N:N）を記載する

## 出力フォーマット

```markdown
# データカタログ

## Entity-Table Mapping

| エンティティ | 物理テーブル | スキーマ | 利用APP | 出典 |
|---|---|---|---|---|

## Column Mapping

### [エンティティ名]
| 論理名 | 物理列名 | データ型 | NULL可 | 備考 |
|---|---|---|---|---|

## Ownership Matrix

| エンティティ | 読取サービス | 書込サービス | 出典 |
|---|---|---|---|
```

## 制約

- 捏造禁止 / オーバーエンジニアリング禁止
