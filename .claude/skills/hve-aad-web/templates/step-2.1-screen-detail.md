# Step 2.1: 画面詳細定義

## このステップの目的

Step 1 の画面一覧に基づき、実装に使える画面ごとの詳細定義書（UX・A11y・セキュリティ・受け入れ基準を含む）を作成する。

## 入力

- `docs/catalog/screen-catalog-APP-*.md`（per-APP 分割された画面カタログ。全 APP 分を集約読みする）
- `docs/catalog/app-catalog.md`（アプリケーション一覧）
- `docs/catalog/persona-screen-catalog.md`（存在すれば必読。`screen-catalog-APP-*.md` の `notes` 列に `common_ref: PSC-XXX` がある画面は本カタログから共通骨格を継承する）
- 推奨（存在すれば）: `docs/catalog/domain-analytics.md`, `docs/catalog/service-catalog.md`, `docs/catalog/data-model.md`, `docs/catalog/service-catalog-matrix.md`, `docs/catalog/test-strategy.md`

## 出力

- `docs/screen/<画面ID>-<画面名>-description.md`（画面ごとに 1 ファイル）

## 実行手順（fan-out）

1. `docs/catalog/screen-catalog-APP-*.md` を全て読み、画面 ID（`{APP-ID}-S###`）と画面名を列挙する。
2. 各画面 ID に対して Agent tool（`subagent_type: hve-architect`）でサブエージェントを並列起動する。このテンプレートの内容をプロンプトとして渡し、`{key}` を対象画面 ID に置換する。
3. 各 fan-out 子は担当画面 1 件のみを処理し、`docs/screen/{key}-<画面名slug>-description.md` を出力する。

## fan-out 子の設計ルール

- アクターごとに別の画面を作成する。
- UX・A11y・セキュリティ・テスト可能な受け入れ基準を含める。
- 参照元ドキュメントと整合させ、不明点は捏造せず TODO / Questions に落とす。
- 共通画面参照ルール: 担当画面の `screen-catalog-APP-*.md` 行の `notes` 列に `common_ref: PSC-XXX` があれば、`docs/catalog/persona-screen-catalog.md` の該当 `persona_screen_id` から共通骨格（操作意味・主要状態・A11y 観点）を継承し、画面定義書「1. 目的と非目的」の冒頭に `共通画面参照: PSC-XXX` を明記する。APP 固有差分のみ各章に展開し、共通骨格と矛盾させない。
  - `common_ref` が無い画面は従来通り単独で作成する。
  - `common_ref: PSC-XXX` があるが `persona-screen-catalog.md` が存在しない、または該当 ID が見つからない場合は、共通骨格を捏造せず `共通画面参照: PSC-XXX（未解決）` と記載する。
- `{screenNameSlug}` は画面名をスラッグ化する（小文字・空白は `-`・英数と `-` のみ）。

## 出力フォーマット

```md
## 1. 目的と非目的
* 所属アプリケーション: APP-xx（`docs/catalog/app-catalog.md` を参照）
* 共通画面参照: PSC-XXX（該当する場合のみ）
* 目的 / 想定ユーザー / 前提

## 2. 画面構成
* レイアウト概要
* コンポーネント一覧（入力/表示/操作）

## 3. ユーザーフロー / 状態
* 主要フロー
* 状態（初期/読込中/空/エラー/完了）

## 4. 入出力・データ
* 表示データ（出典: data-model / service-catalog）
* 入力項目（型/必須/制約）
* API連携予定（未確定は TODO として明示。捏造しない）

## 5. バリデーション & エラーメッセージ
* ルール
* 文言（日本語）

## 6. A11y / i18n
* キーボード操作 / フォーカス順 / aria / 読み上げ / 色以外の表現

## 7. セキュリティ / プライバシー
* 取り扱うデータ分類（個人情報等）
* サニタイズ/制限
* 保存範囲（ローカル/送信有無）

## 8. 非機能要件
* パフォーマンス/レスポンス目安（根拠が無ければ TODO）
* 監視やログ（必要時のみ）

## 9. 受け入れ基準（テスト可能）
* Given/When/Then 形式で 3〜7個
```

## 完了条件

- 画面定義書が画面一覧に基づいて全て作成されている。

---

**捏造禁止**: ID・URL・数値・固有名を根拠なく生成しない。不明は `TBD` または `不明（要確認）` と明記する。
**オーバーエンジニアリング禁止**: 指示にない未来予測的な汎用化・抽象化・将来拡張点の先回り追加を行わない。YAGNI に従う。
