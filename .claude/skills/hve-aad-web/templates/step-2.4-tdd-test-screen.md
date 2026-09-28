# Step 2.4: 画面 TDD テストスペック

## このステップの目的

テスト戦略書と画面定義書を根拠に、TDD Red フェーズ用のテスト仕様書を画面ごとに作成する。

## 入力

- `docs/catalog/test-strategy.md`
- `docs/screen/<画面ID>-<画面名>-description.md`
- `docs/catalog/screen-catalog-APP-*.md`
- `docs/catalog/data-model.md`
- `docs/catalog/domain-analytics.md`
- `docs/catalog/app-catalog.md`（アプリケーション一覧）

## 出力

- `docs/test-specs/<screenId>-test-spec.md`（画面ごとに 1 ファイル）

## 依存

- Step 2.1（画面定義書）が全て完了していること。

## 実行手順（fan-out）

1. `docs/screen/*-description.md` から画面 ID を列挙する。
2. 各画面 ID に対して Agent tool（`subagent_type: hve-architect`）でサブエージェントを並列起動する。このテンプレートの内容をプロンプトとして渡し、`{key}` を対象画面 ID に置換する。
3. 各 fan-out 子は担当画面 1 件のみを処理し、`docs/test-specs/{key}-test-spec.md` を出力する。

## fan-out 子の設計ルール

- 画面定義書から操作・バリデーション・API 呼び出しを抽出し、テストケースに反映する。
- AC-ID ↔ Test-ID の双方向トレーサビリティ表を必須記載する。未確定 ID は `TBD（要確認）` として両方向表に同値反映する。
- 未確定 API 契約は PASS 必須の実行テストにせず、Questions / 残リスクに記録する。
- 各テストケースに実行環境、外部サービス要否、必要な環境変数・設定ファイルを記載する。
- UI カテゴリでは単体テスト（コンポーネント）と E2E（操作シナリオ）の両観点を含める。
- 秘密情報をテスト仕様に含めない。

## 出力フォーマット

必須セクション:

1. 概要
2. ATDD(UI)
3. E2E / 操作シナリオ（`実行環境` / `外部サービス要否` / `必要設定` 列を含める）
4. AC→Test トレーサビリティ
5. バリデーション（`実行環境` / `外部サービス要否` / `必要設定` 列を含める）
6. テストデータ
7. API モック / ダブル
8. API 契約検証
9. A11y
10. TDD 順序
11. 網羅性
12. Questions
13. Test→AC 逆引き

## 完了条件

- テスト仕様書が画面カタログに基づいて全画面分作成されている。

---

**捏造禁止**: ID・URL・数値・固有名を根拠なく生成しない。不明は `TBD` または `不明（要確認）` と明記する。
**オーバーエンジニアリング禁止**: 指示にない未来予測的な汎用化・抽象化・将来拡張点の先回り追加を行わない。YAGNI に従う。
