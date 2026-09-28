# Step 8: ペルソナ画面カタログ

## 目的

ペルソナカタログを根拠に、ペルソナ別の共通画面骨格を抽出する。
複数 APP-ID で同一ペルソナが共通利用する画面（ダッシュボード・通知一覧・プロフィール等）の重複定義を防ぐ。

## 入力

- `docs/catalog/persona-catalog.md`（必須）
- `docs/catalog/app-catalog.md`（必須）
- `docs/catalog/domain-analytics.md`（補助）
- `docs/catalog/service-catalog.md`（補助）

## 出力

- `docs/catalog/persona-screen-catalog.md`

## 抽出ルール

- ペルソナごとに、**2 つ以上の APP-ID** で共通利用される画面骨格のみを抽出する
- 1 APP-ID 専用の画面は含めない（AAD-WEB の per-APP 画面カタログに委譲）
- 根拠が不明な場合は共通画面に含めず、「候補/TBD」表に記録する

## ID 採番

- `PSC-XXX` 形式（PSC = Persona Screen Catalog）
- 下流の AAD-WEB（画面詳細設計）からこの ID で参照される

## 出力フォーマット

```markdown
# ペルソナ画面カタログ

| persona_screen_id | 画面名 | 目的 | persona_id | shared_by_apps | 出典 |
|---|---|---|---|---|---|
| PSC-001 | ダッシュボード | [目的] | PRS-001 | APP-001, APP-002 | [出典] |

### 候補/TBD

根拠不足の共通画面候補:

| 画面名 | 候補理由 | 不足している根拠 |
|---|---|---|
```

## 制約

- 推測で共通化しない
- 不明な項目は TBD と明記する
- 捏造禁止 / オーバーエンジニアリング禁止
