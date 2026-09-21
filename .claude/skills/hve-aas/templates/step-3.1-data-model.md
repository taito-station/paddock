# Step 3.1: データモデリング

## 目的

ドメイン分析結果とサービス一覧を根拠に、データモデル（概念モデル + 物理マッピング）を設計する。
Web アプリのデータベースに加え、データフロー型アプリのデータソース/デスティネーション/中間データも統合する。

## 入力

- `docs/catalog/domain-analytics.md`（必須）
- `docs/catalog/service-catalog.md`（必須）
- `docs/catalog/app-catalog.md`（必須）

## 出力

- `docs/catalog/data-model.md`（常に必須 — 索引/統合版）
- 50,000 文字超の場合のみ、以下の sidecar 3 件を全て作成:
  - `docs/catalog/data-model-service-stores.md`
  - `docs/catalog/data-model-consistency-events.md`
  - `docs/catalog/data-model-diagrams.md`

## 実行手順

1. ドメイン分析の集約・エンティティからエンティティカタログを作成する
2. 各エンティティに物理マッピング（テーブル/コレクション等）を設計する
3. 各エンティティに「利用 APP」（N:N）を記載する
4. データフロー型 APP-ID がある場合:
   - データフロー上の役割（ソース / デスティネーション / 中間）を明記する
   - 外部システム自体をエンティティにしない（授受される業務データを登録する）

## 分割ルール

- 50,000 文字超: 親ファイルに固定見出しと統合ビューを残し、sidecar 3 件に分割する
- 親から各 sidecar へのリンク、sidecar から親への戻りリンクを必ず保持する
- sidecar は上記 3 件のみ（追加しない）

## 制約

- 不明な項目は推測せず TBD と明記する
- 捏造禁止 / オーバーエンジニアリング禁止
