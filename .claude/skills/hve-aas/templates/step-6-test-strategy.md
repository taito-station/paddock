# Step 6: TDD テスト戦略

## 目的

サービスカタログの API 一覧・依存関係マトリクス・データモデルを根拠に、TDD のためのプロジェクト全体テスト戦略書を作成する。

## 入力

- `docs/catalog/service-catalog-matrix.md`（必須）
- `docs/catalog/data-model.md`（必須）
- `docs/catalog/domain-analytics.md`（必須）
- `docs/catalog/service-catalog.md`（必須）
- `docs/catalog/app-catalog.md`（必須）
- `docs/catalog/data-catalog.md`（存在すれば参照）

## 出力

- `docs/catalog/test-strategy.md`

## 記載内容

### テスト分類定義

- 単体テスト（Unit Test）
- 統合テスト（Integration Test）
- E2E テスト
- データフロー型 APP-ID がある場合: バッチ固有テスト種別

### テストダブル戦略

- Mock/Stub/Fake の使い分け方針
- 外部サービス依存のテスト方針

### Polyglot Persistence テスト方針

- データストアごとのテスト方針
- データ品質・ストレージ観点

### バッチテスト方針（条件付き）

データフロー型 APP-ID が含まれる場合のみ:
- 冪等性テスト方針
- データ品質テスト方針
- 大量データテスト方針
- 障害注入・復旧テスト方針

### 網羅性チェック

- サービスごとのテストカバレッジ方針
- APP-ID ごとのテスト対象範囲

## 制約

- 不明な項目は推測せず TBD と明記する
- 捏造禁止 / オーバーエンジニアリング禁止
