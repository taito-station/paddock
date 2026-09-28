# Step 1: 画面一覧・遷移設計

## このステップの目的

ドメイン分析・サービス一覧・データモデルを根拠に、APP-ID ごとの画面一覧（表）と画面遷移図（Mermaid）を設計する。アクターの「人」ごとに画面を分け、ポータル（タブ）から主要機能へ到達できる構造にする。

## 入力

- `docs/catalog/domain-analytics.md`
- `docs/catalog/service-catalog.md`（機能・責務の補助）
- `docs/catalog/data-model.md`（表示・入力項目の補助）
- `docs/catalog/app-catalog.md`（アプリケーション一覧。**必須**。各画面がどの APP-ID に所属するかの判定根拠）
- `docs/catalog/persona-screen-catalog.md`（存在する場合のみ。AAS Step 8 で生成されるペルソナ別共通画面骨格）

## 出力

- `docs/catalog/screen-catalog-{APP-ID}.md`（**APP-ID 単位で分割**。担当 APP の画面のみを書く）

旧形式の単一ファイル `docs/catalog/screen-catalog.md` は作成しない。

## 実行手順（fan-out）

1. `docs/catalog/app-catalog.md` から APP-ID の一覧を列挙する。
2. 各 APP-ID に対して Agent tool（`subagent_type: hve-architect`）でサブエージェントを並列起動する。このテンプレートの内容をプロンプトとして渡し、`{key}` を対象 APP-ID に置換する。
3. 各 fan-out 子は **担当 APP-ID 1 つ分のみ** を扱い、他 APP の画面行を絶対に含めない（並列実行での衝突防止）。

## fan-out 子の設計ルール

- 人のアクターごとに別の画面を作成する。アクターの「人」以外は作成しない。
- 画面上の表示文言にユースケース ID をそのまま出さない（人間可読な名称を使う）。
- `screen_id` は安定採番する（例: `{APP-ID}-S001`, `{APP-ID}-S002`...）。ユースケース ID を埋め込まない。
- 1 画面は必ず 1 つの APP-ID に所属する（1:1 関係）。
- 既に `screen-catalog-{APP-ID}.md` が存在する場合は、既存 `screen_id` を維持し差分更新する。追加画面のみ末尾に採番追加する（欠番は詰めない）。
- ポータル画面はタブ形式とし、主要ユースケース・主要機能へ到達できる遷移を必ず持つ。
- `docs/catalog/persona-screen-catalog.md` が存在する場合、そこに記載された共通画面骨格を本 APP 用に再定義しない。共通画面については `notes` 列に `common_ref: PSC-XXX` を記載する。APP 固有差分がある場合のみ短く追記する。共通画面骨格自体を再生成・上書きしてはならない（詳細な画面定義は Step 2.1 で行う）。
- `persona-screen-catalog.md` が存在しない場合は独自に画面一覧を生成する。

## 出力フォーマット

### 画面一覧（Screen List）

Markdown 表（列固定）:

| screen_id | screen_name | 所属APP | description | function_type | notes |
| --------- | ----------- | ------- | ----------- | ------------- | ----- |

- `notes`: 根拠（参照ファイル）・不明点・要確認を短く。欠損は空欄可。

### 画面遷移図（Screen Transition Diagram）

- Mermaid `flowchart TD` を使う。
- 起点は `Portal (tabs)`。
- 画面数が多い場合はタブ・機能単位で `subgraph` を使い分割する。

### 注意事項（Assumptions / Open Questions）

- 断定できない点、矛盾、要確認を箇条書きにする。
- 質問が必要なら最大 3 点まで（同時に暫定案も書く）。

## 完了条件

- `docs/catalog/screen-catalog-{APP-ID}.md` が作成されている。

---

**捏造禁止**: ID・URL・数値・固有名を根拠なく生成しない。不明は `TBD` または `不明（要確認）` と明記する。
**オーバーエンジニアリング禁止**: 指示にない未来予測的な汎用化・抽象化・将来拡張点の先回り追加を行わない。YAGNI に従う。
