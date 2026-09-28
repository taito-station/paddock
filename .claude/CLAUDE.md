# CLAUDE.md — HVE Design Methodology

HypervelocityEngineering (HVE) の設計手法を Claude Code ネイティブで実行するためのプロジェクト設定。

出典: [dahatake/HypervelocityEngineering](https://github.com/dahatake/HypervelocityEngineering) (MIT License)

## 方法論

ビジネス要件の定義からソフトウェア詳細設計までを 3 つのワークフローで段階的に進める。

```
ARD (要件定義) → AAS (アーキテクチャ設計) → AAD-WEB (Web 詳細設計)
```

各ワークフローは `/hve-ard`, `/hve-aas`, `/hve-aad-web` スキルで順次実行する。

## 出力ディレクトリ構造

```
docs/
├── catalog/                    # カタログ類（ユースケース、サービス、データモデル等）
│   ├── use-case-catalog.md
│   ├── app-catalog.md
│   ├── domain-analytics.md
│   ├── service-catalog.md
│   ├── data-model.md
│   ├── service-catalog-matrix.md
│   ├── test-strategy.md
│   ├── persona-catalog.md
│   ├── persona-screen-catalog.md
│   └── screen-catalog-APP-*.md
├── screen/                     # 画面定義書
│   └── <画面ID>-<画面名>-description.md
├── services/                   # サービス定義書
│   └── <serviceId>-<serviceName>-description.md
├── test-specs/                 # テストスペック
│   └── <ID>-test-spec.md
├── usecase/                    # ユースケース詳細
│   └── <UC-ID>-detail.md
├── business/                   # 事業分析
│   └── <BIZ-ID>-analysis.md
└── business-requirement.md     # 事業要件書
work/                           # 一時作業ファイル（成果物に含めない）
```

## 規律の優先順位

1. **捏造禁止**（最優先）— 根拠のない ID/URL/名称/数値を作らない
2. **オーバーエンジニアリング禁止** — YAGNI、不要な抽象化を作らない
3. **最小差分原則** — 必要な変更だけを行う

## 出力言語

日本語。見出し＋箇条書き中心で簡潔に。

## 成果物の完了基準

各ステップの成果物には以下を含めること:
- 目的
- 変更点（箇条書き）
- 影響範囲
- 検証結果
- 既知の制約
- 次のステップ
