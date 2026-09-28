---
name: hve-aas
description: |
  App Architecture Selection (AAS) ワークフロー。
  ARD の出力からアーキテクチャ選定・ドメイン分析・データモデル・テスト戦略までを実行する。
  「AAS」「アーキテクチャ設計」「ドメイン分析」で起動。
---

# AAS — App Architecture Selection

ARD の成果物（アプリケーションカタログ・ユースケースカタログ・要件定義書）を入力に、
アーキテクチャ設計・ドメイン分析・データモデリング・テスト戦略を実行する。

## 前提条件

以下のファイルが存在すること（ARD 完了済み）:

- `docs/catalog/app-catalog.md`
- `docs/catalog/use-case-catalog.md`
- `docs/architectural-requirements-app-*.md`（対象 APP ごと）

## パラメータ

なし（ARD の出力ファイルを自動参照する）。

## DAG（ステップ依存関係）

```
Step 1 (アーキテクチャ選定)
  └→ Step 2.1 (ドメイン分析)
       └→ Step 2.2 (サービス識別)
            └→ Step 3.1 (データモデル)
                 ├→ Step 3.2 (サンプルデータ)
                 └→ Step 4 (データカタログ)
                      └→ Step 5 (サービスカタログマトリクス)
                           └→ Step 6 (テスト戦略)
                                └→ Step 7 (ペルソナカタログ)
                                     └→ Step 8 (ペルソナ画面カタログ)
```

全ステップ逐次実行（fan-out なし）。

## ステップ一覧

| Step | 目的 | 出力 | テンプレート |
|---|---|---|---|
| 1 | アーキテクチャ候補選定 | `docs/catalog/app-arch-catalog.md` | `templates/step-1-arch-selection.md` |
| 2.1 | DDD ドメイン分析 | `docs/catalog/domain-analytics.md` | `templates/step-2.1-domain-analysis.md` |
| 2.2 | サービス識別 | `docs/catalog/service-catalog.md` | `templates/step-2.2-service-identify.md` |
| 3.1 | データモデリング | `docs/catalog/data-model.md` | `templates/step-3.1-data-model.md` |
| 3.2 | サンプルデータ生成 | `src/data/sample-data.json` | `templates/step-3.2-sample-data.md` |
| 4 | データカタログ | `docs/catalog/data-catalog.md` | `templates/step-4-data-catalog.md` |
| 5 | サービスカタログマトリクス | `docs/catalog/service-catalog-matrix.md` | `templates/step-5-service-catalog-matrix.md` |
| 6 | TDD テスト戦略 | `docs/catalog/test-strategy.md` | `templates/step-6-test-strategy.md` |
| 7 | ペルソナカタログ | `docs/catalog/persona-catalog.md` | `templates/step-7-persona-catalog.md` |
| 8 | ペルソナ画面カタログ | `docs/catalog/persona-screen-catalog.md` | `templates/step-8-persona-screen.md` |

## 実行手順

各ステップについて:

1. テンプレートファイルを Read して詳細なプロンプトを確認する
2. 入力ファイルの存在を確認する（欠落時は TBD を記載して続行、ただし必須入力の欠落は停止）
3. テンプレートの指示に従って成果物を作成する
4. 出力ファイルが正しく作成されたことを確認する
5. 完了報告（目的/変更点/影響範囲/検証結果/既知の制約/次のステップ）を行う
6. 次のステップに進む

## 注意事項

- 各ステップは前のステップの出力に依存するため、逐次実行すること
- `docs/catalog/app-catalog.md` の APP-ID を横断的に参照し、成果物間の整合性を維持する
- 50,000 文字超の成果物は分割する（特にデータモデル: data-model.md + sidecar 3 件）
