# Step 2.3: サービス TDD テストスペック

## このステップの目的

テスト戦略書とサービス定義書を根拠に、TDD Red フェーズ用のテスト仕様書をサービスごとに作成する。

## 入力

- `docs/catalog/test-strategy.md`
- `docs/services/<serviceId>-<serviceName>-description.md`
- `docs/catalog/service-catalog-matrix.md`
- `docs/catalog/data-model.md`
- `docs/catalog/domain-analytics.md`
- `docs/catalog/app-catalog.md`（アプリケーション一覧）

## 出力

- `docs/test-specs/<serviceId>-test-spec.md`（サービスごとに 1 ファイル）

## 依存

- Step 2.2（マイクロサービス定義書）が全て完了していること。

## 実行手順（fan-out）

1. `docs/services/*-description.md` からサービス ID を列挙する。
2. 各サービス ID に対して Agent tool（`subagent_type: hve-architect`）でサブエージェントを並列起動する。このテンプレートの内容をプロンプトとして渡し、`{key}` を対象サービス ID に置換する。
3. 各 fan-out 子は担当サービス 1 件のみを処理し、`docs/test-specs/{key}-test-spec.md` を出力する。

## fan-out 子の設計ルール

- `docs/catalog/test-strategy.md` からテスト分類・テストダブル方針・データストア別方針を抽出する。
- サービス定義書から API・依存・イベントを抽出し、テストケースに反映する。
- AC-ID ↔ Test-ID の双方向トレーサビリティ表を必須記載する。未確定 ID は `TBD（要確認）` として両方向表に同値反映する。
- 未確定契約（正式 API ID / path / event / schema / enum 値が未確定）は PASS 必須の実行テストにせず、Questions / 残リスクに記録する。契約確定後に Contract test 化する。
- 各テストケースに実行環境（ローカル / CI / デプロイ先）、外部サービス要否、必要な環境変数・設定ファイルを記載する。
- Unit / 実装コード向け TDD RED/GREEN はローカル実行可能を既定とし、外部 I/O は Mock / Stub / Emulator 等へ切り分ける。
- 接続文字列・アカウントキー・トークン等の秘密情報をテスト仕様に含めない。

## 出力フォーマット

必須セクション:

1. 概要
2. ATDD(API)
3. テストケース表（`実行環境` / `外部サービス要否` / `必要設定` 列を含める）
4. AC→Test トレーサビリティ
5. テストデータ
6. テストダブル
7. 契約テスト
8. TDD 順序
9. 網羅性
10. Questions
11. Test→AC 逆引き

## 完了条件

- テスト仕様書がサービスカタログに基づいて全サービス分作成されている。

---

**捏造禁止**: ID・URL・数値・固有名を根拠なく生成しない。不明は `TBD` または `不明（要確認）` と明記する。
**オーバーエンジニアリング禁止**: 指示にない未来予測的な汎用化・抽象化・将来拡張点の先回り追加を行わない。YAGNI に従う。
