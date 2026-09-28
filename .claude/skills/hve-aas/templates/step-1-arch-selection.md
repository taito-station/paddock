# Step 1: アーキテクチャ候補選定

## 目的

アプリケーションリストの各 APP について、APP 別要件定義書を根拠に固定候補からアーキテクチャを選定する。

## 入力

- `docs/catalog/app-catalog.md`（必須）
- `docs/architectural-requirements-app-NNN.md`（対象 APP ごとに必須）

## 出力

- `docs/catalog/app-arch-catalog.md`

## 固定アーキテクチャ候補（12 種）

| # | 候補 | 概要 |
|---|---|---|
| 1 | Web + Cloud | クラウドホスティングの Web アプリケーション |
| 2 | Web + OnPrem | オンプレミスの Web アプリケーション |
| 3 | Mobile + Cloud | クラウドバックエンドのモバイルアプリ |
| 4 | Mobile + OnPrem | オンプレミスバックエンドのモバイルアプリ |
| 5 | Desktop + Cloud | クラウド連携のデスクトップアプリ |
| 6 | Desktop + OnPrem | オンプレミスのデスクトップアプリ |
| 7 | Standalone PC | スタンドアロンの PC アプリケーション |
| 8 | Embedded | 組み込みシステム |
| 9 | IoT + Cloud | クラウド連携の IoT システム |
| 10 | IoT + Edge + Cloud | エッジコンピューティング + クラウドの IoT |
| 11 | Hybrid Cloud | ハイブリッドクラウド構成 |
| 12 | Data Pipeline | データパイプライン・バッチ処理 |

## 実行手順

1. `docs/catalog/app-catalog.md` から全 APP-ID を取得する
2. 各 APP-ID の要件定義書 `docs/architectural-requirements-app-NNN.md` を読む
3. 要件に基づき、12 候補から最適なアーキテクチャを選定する
4. スコアリング: 各評価軸に対して ◎(3点) / ○(2点) / △(1点) で評価する
5. 下流分類として `web-cloud` または `batch` を付与する

## 出力フォーマット

```markdown
# アプリケーションアーキテクチャカタログ

## APP-001: [アプリ名]

### 選定結果
- 推奨アーキテクチャ: [候補名]
- 下流分類: web-cloud / batch
- 総合スコア: XX点

### 評価マトリクス
| 評価軸 | 重み | 候補A | 候補B | 候補C |
|---|---|---|---|---|
| [軸名] | [重み] | ◎/○/△ | ◎/○/△ | ◎/○/△ |

### 選定根拠
- [根拠の箇条書き、出典付き]
```

## 制約

- 要件定義書の欠落、APP-ID 不一致がある場合は対象 APP を停止する（fail-closed）
- 必須入力を他の属性や固定候補から補完しない
- 捏造禁止 / オーバーエンジニアリング禁止
