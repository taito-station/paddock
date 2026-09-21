---
name: hve-ard
description: |
  Auto Requirement Definition (ARD) ワークフロー。
  企業名から事業分析・ユースケース抽出・アプリケーション分類・要件定義までを実行する。
  「ARD」「要件定義」「事業分析」で起動。
---

# ARD（Auto Requirement Definition）ワークフロー

企業の事業分析からソフトウェア要件定義までを段階的に進めるワークフロー。
HVE 方法論における最初のフェーズ（要件定義）を担う。

## このワークフローが出力するもの

1. 企業の事業ポートフォリオ分析・事業要件文書
2. ソフトウェア化対象のユースケースカタログ
3. ユースケースを束ねたアプリケーションカタログ（アーキタイプ単位）
4. アプリケーションごとの要求定義書

すべて `docs/` 配下に Markdown で出力する。ディレクトリ構造は `CLAUDE.md` の出力ディレクトリ構造に従う。

## パラメータ

| パラメータ | 必須 | 説明 |
|---|---|---|
| `company_name` | 必須 | 対象企業名 |
| `target_business` | 任意 | 分析対象の事業・業務名（文章 or 資料パス）。指定時は事業ポートフォリオ分析（Step 1/1.1/1.2）を **スキップ** し、Step 2 から開始する |
| `survey_base_date` | 任意 | 調査基準日 |
| `survey_period_years` | 任意 | 調査対象期間（年数、例: `10`, `30`） |
| `target_region` | 任意 | 対象地域（例: `国内事業のみ`, `グローバル全体`） |
| `analysis_purpose` | 任意 | 分析目的（例: `中長期成長戦略の立案`, `収益性改善`） |
| `attached_docs` | 任意 | 一次情報として最優先で参照する添付資料。指定がある場合はファイル名を推測せず与えられたパスをそのまま読む |
| `include_kpi_okr` | 任意（既定 `false`） | Step 2.1（KPI/OKR 定義）を実行するかどうか |

未指定のパラメータは各ステップのテンプレート内で `TBD` / 既定の扱いに従う。値を推測して埋めてはならない。

## ワークフロー全体像（DAG）

```
[target_business 未指定の場合]                    [target_business 指定の場合]
Step 1: 事業ポートフォリオスキャン                  Step 2: ターゲット事業分析 ─────┐
   │ (docs/company-business-recommendation.md)                                    │
   ▼ fan-out（BIZ-NN ごとに並列）                                                   │
Step 1.1: 事業深掘り × N                                                           │
   │ (docs/business/{BIZ-NN}-analysis.md)                                         │
   ▼ join                                                                         │
Step 1.2: 事業分析統合 ────────────────────────────────────────────────────────────┘
   │ (docs/company-business-requirement.md)              (docs/business-requirement.md)
   └───────────────────────────────┬───────────────────────────────────────────────┘
                                    ▼
                    Step 2.1: KPI/OKR 定義（任意, include_kpi_okr=true のときのみ）
                          (docs/recommended-kpi-okr.md)
                                    │
                                    ▼
                    Step 3.1: ユースケース骨格抽出
                          (docs/catalog/use-case-skeleton.md)
                                    │
                                    ▼ fan-out（UC ごとに並列）
                    Step 3.2: ユースケース詳細 × N
                          (docs/usecase/{UC-ID}-detail.md)
                                    │
                                    ▼ join
                    Step 3.3: ユースケースカタログ統合
                          (docs/catalog/use-case-catalog.md)
                                    │
                                    ▼
                    Step 4.1: アプリケーションリスト作成
                          (docs/catalog/app-catalog.md)
                                    │
                                    ▼
                    Step 4.2: 個別 APP 要件定義（単一エージェントで順次処理、fan-out しない）
                          (docs/architectural-requirements-app-NNN.md)
```

- `target_business` が指定されている場合、Step 1 / 1.1 / 1.2 はスキップし、Step 2 から開始する。
- Step 2.1 は既定で **実行しない**。ユーザーが `include_kpi_okr=true` を明示した場合のみ実行する。
- Step 2.1 の出力（KPI/OKR）は Step 3.1 / 3.2 / 4.1 から **任意参照**（存在すれば使う、無ければ省略。捏造しない）。

## 実行手順

各ステップの詳細な実行指示・出力フォーマットは `templates/` 配下の対応ファイルに定義されている。
ステップを実行する際は該当テンプレートを Read してから、その指示に従うこと（本 SKILL.md には要旨のみを記載する）。

### Step 1: 事業ポートフォリオスキャン（`target_business` 未指定時のみ）

- テンプレート: `templates/step-1-business-scan.md`
- 目的: 企業全体を俯瞰し、深掘り対象となる事業分野候補を `BIZ-NN` 形式で列挙する
- 入力: `company_name`, `survey_base_date`, `survey_period_years`, `target_region`, `analysis_purpose`, `attached_docs`
- 出力: `docs/company-business-recommendation.md`

### Step 1.1: 事業深掘り（fan-out）

- テンプレート: `templates/step-1.1-business-deepdive.md`
- 目的: Step 1 で列挙した各事業候補について個別に深掘り分析を行う
- 手順:
  1. `docs/company-business-recommendation.md` を読み、`BIZ-NN` の一覧を取得する
  2. 各 `BIZ-NN` について、`templates/step-1.1-business-deepdive.md` の内容を指示として与えた Agent tool のサブエージェントを **並列起動** する（1 事業候補 = 1 サブエージェント）
  3. 各サブエージェントには対象の `{key}`（`BIZ-NN`）のみを与え、他の候補には言及させない
- 出力: `docs/business/{BIZ-NN}-analysis.md`（候補ごとに 1 ファイル）

### Step 1.2: 事業分析統合（join）

- テンプレート: `templates/step-1.2-business-join.md`
- 目的: Step 1.1 の全出力を統合し、企業全体の事業要件文書を生成する
- 手順: `docs/business/*-analysis.md` を **全て読み**、`docs/company-business-recommendation.md` と合わせて統合レポートを作成する
- 出力: `docs/company-business-requirement.md`

### Step 2: ターゲット事業分析（`target_business` 指定時はここから開始）

- テンプレート: `templates/step-2-targeted-analysis.md`
- 目的: 指定された事業・業務について As-Is / To-Be / Gap / Strategic Recommendations を分析する
- 入力: `company_name`（任意）, `target_business`（必須）, `survey_period_years`, `target_region`, `analysis_purpose`, `attached_docs`, `docs/company-business-requirement.md`（Step 1.2 が先行実行されている場合のみ参考として参照）
- 出力: `docs/business-requirement.md`

### Step 2.1: KPI/OKR 定義（任意、`include_kpi_okr=true` の場合のみ）

- テンプレート: `templates/step-2.1-kpi-okr.md`
- 目的: 事業要件文書の戦略的記述を根拠に KPI/OKR とデータ収集設計を定義する
- 入力: `docs/business-requirement.md`（優先）または `docs/company-business-requirement.md`（フォールバック）
- 出力: `docs/recommended-kpi-okr.md`

### Step 3.1: ユースケース骨格抽出

- テンプレート: `templates/step-3.1-usecase-skeleton.md`
- 目的: 事業要件文書からソフトウェア化対象のユースケースを `UC-*` 形式で骨格レベルに抽出する
- 入力: `docs/business-requirement.md`（優先）または `docs/company-business-requirement.md`（フォールバック）、`docs/recommended-kpi-okr.md`（存在すれば任意参照）
- 出力: `docs/catalog/use-case-skeleton.md`

### Step 3.2: ユースケース詳細（fan-out）

- テンプレート: `templates/step-3.2-usecase-detail.md`
- 目的: Step 3.1 の各ユースケースについて詳細仕様を生成する
- 手順:
  1. `docs/catalog/use-case-skeleton.md` を読み、`UC-*` の一覧を取得する
  2. 各 `UC-*` について、`templates/step-3.2-usecase-detail.md` の内容を指示として与えた Agent tool のサブエージェントを **並列起動** する（1 ユースケース = 1 サブエージェント）
  3. 各サブエージェントには対象の `{key}`（`UC-*`）のみを与え、他のユースケースには言及させない
- 出力: `docs/usecase/{UC-ID}-detail.md`（ユースケースごとに 1 ファイル）

### Step 3.3: ユースケースカタログ統合（join）

- テンプレート: `templates/step-3.3-usecase-join.md`
- 目的: Step 3.2 の全出力を ID 順に統合した最終カタログを生成する
- 手順: `docs/usecase/*-detail.md` を **全て読み**、`docs/catalog/use-case-skeleton.md` の並び順を根拠に統合する
- 出力: `docs/catalog/use-case-catalog.md`

### Step 4.1: アプリケーションリスト作成

- テンプレート: `templates/step-4.1-app-list.md`
- 目的: ユースケースを実装手段（新規導入／既存拡張／連携／業務改革／組織改革）で仕分けし、アプリケーション（アーキタイプ）単位に束ねる
- 入力: `docs/catalog/use-case-catalog.md`、`docs/recommended-kpi-okr.md`（存在すれば任意参照）
- 出力: `docs/catalog/app-catalog.md`

### Step 4.2: 個別 APP 要件定義（順次処理、fan-out しない）

- テンプレート: `templates/step-4.2-app-requirements.md`
- 目的: `docs/catalog/app-catalog.md` に列挙された全 APP について要求定義書を upsert する
- 手順: **単一セッション内で APP を出現順に処理する**。Step 3.2 とは異なり fan-out しない（既存 confirmed 内容の上書き事故を避けるため）
- 出力: `docs/architectural-requirements-app-NNN.md`（APP ごとに 1 ファイル）

## 共通の注意事項

- **捏造禁止**: ID・URL・固有名・数値・事実を根拠なく作らない。不明・未確認は `TBD` / `要確認` と明示する。
- **オーバーエンジニアリング禁止**: 指示にない汎用化・抽象化・将来拡張の先回りをしない。
- 既に `docs/` 配下に存在する成果物は、明示的にそのステップの入力として指定されない限り変更しない。
- 各ステップの詳細な出力フォーマット・章立て・品質基準は `templates/` 配下の該当ファイルを Read して確認すること。本 SKILL.md には要旨のみを記載しており、実行時は必ずテンプレート本文に従う。
