# Step 2.2: マイクロサービス詳細定義

## このステップの目的

サービスカタログおよび AAS の各種成果物を根拠に、マイクロサービスごとの詳細定義書（API・イベント・データ・セキュリティ）を作成する。

## 入力

- `docs/catalog/service-catalog.md`
- `docs/catalog/domain-analytics.md`
- `docs/catalog/service-catalog-matrix.md`
- `docs/catalog/data-model.md`
- `docs/catalog/app-catalog.md`（アプリケーション一覧。サービスが属する APP-ID の判定根拠）
- 推奨（存在すれば）: `docs/catalog/test-strategy.md`

## 出力

- `docs/services/<serviceId>-<serviceName>-description.md`（サービスごとに 1 ファイル）

## 実行手順（fan-out）

1. `docs/catalog/service-catalog.md` からサービス ID（`SVC-*`）と名称を列挙する。
2. 各サービス ID に対して Agent tool（`subagent_type: hve-architect`）でサブエージェントを並列起動する。このテンプレートの内容をプロンプトとして渡し、`{key}` を対象サービス ID に置換する。
3. 各 fan-out 子は担当サービス 1 件のみを処理し、`docs/services/{key}-<serviceNameSlug>-description.md` を出力する。slug が不明な場合は `{key}-description.md` で可。

## fan-out 子の設計ルール

- 各マイクロサービス定義書の「サービスメタ情報」に「利用アプリケーション」（N:N）を記載する（`docs/catalog/app-catalog.md` を参照）。
- 推測はしない。不明は `TBD` とし、根拠・理由を添える。
- サンプルデータの具体値は転記しない（要約のみ）。
- 対象外サービスに変更を入れない。

## 出力フォーマット

```md
## 1. サービスメタ情報
* サービスID / サービス名
* 利用アプリケーション（APP-ID、N:N）
* 責務概要

## 2. API 仕様
* エンドポイント一覧（method / path / 概要）
* リクエスト/レスポンス（型・必須・制約）
* 認証・認可

## 3. イベント仕様
* Publish/Subscribe するイベント（あれば）
* スキーマ概要

## 4. データ仕様
* 所有エンティティ（data-model.md との対応）
* 永続化方式（判明していれば）

## 5. セキュリティ
* 取り扱うデータ分類
* 認可・アクセス制御
* 秘密情報の扱い

## 6. 非機能要件
* パフォーマンス/可用性目安（根拠が無ければ TODO）
* 監視・ログ（必要時のみ）

## 7. 依存関係
* 依存する他サービス / 外部連携
```

## 完了条件

- マイクロサービス定義書がサービスカタログに基づいて全て作成されている。

---

**捏造禁止**: ID・URL・数値・固有名を根拠なく生成しない。不明は `TBD` または `不明（要確認）` と明記する。
**オーバーエンジニアリング禁止**: 指示にない未来予測的な汎用化・抽象化・将来拡張点の先回り追加を行わない。YAGNI に従う。
