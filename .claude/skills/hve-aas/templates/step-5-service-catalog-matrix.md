# Step 5: サービスカタログマトリクス

## 目的

サービス一覧、データモデル、画面一覧、ドメイン分析を統合して、画面 → API → データのマッピングを作成する。
データフロー型アプリのジョブ DAG・スケジュール・リトライ戦略も統合する。

## 入力

- `docs/catalog/service-catalog.md`（必須）
- `docs/catalog/data-model.md`（必須）
- `docs/catalog/screen-catalog-APP-*.md`（存在すれば参照）
- `docs/catalog/domain-analytics.md`（必須）
- `docs/catalog/app-catalog.md`（必須）

## 出力

- `docs/catalog/service-catalog-matrix.md`

## 出力構成

### Table A: 画面 → API マッピング

| 画面ID | 画面名 | 呼出API | HTTP Method | 所属APP |
|---|---|---|---|---|

### Table B: API → データ マッピング

| API | 読取エンティティ | 書込エンティティ | トランザクション境界 |
|---|---|---|---|

### Table C: サービス責務マトリクス

| SVC-ID | サービス名 | 責務 | 依存サービス | 利用APP | 出典 |
|---|---|---|---|---|---|

### Table D: ジョブ実行制御マトリクス（条件付き）

データフロー型 APP-ID が存在する場合のみ作成:

| APP-ID | Job-ID | ジョブ名 | 上流Job | 下流Job | 起動条件 | リトライ概要 | 冪等性 | 出典 |
|---|---|---|---|---|---|---|---|---|

データフロー型 APP-ID が存在しない場合、Table D は作成しない。

## 制約

- 不明な項目は推測せず TBD と明記する
- 捏造禁止 / オーバーエンジニアリング禁止
