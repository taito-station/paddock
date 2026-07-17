# qa — 質問票 + 回答（knowledge の入力）

`docs/original-docs/` の一次資料や調査中に生じた**確認すべき質問**と、その**回答**を蓄える中間層。
回答済みの qa が `docs/knowledge/`（と `docs/specifications/`）への差分マージの入力になる。
全体像は [docs/knowledge/README.md](../knowledge/README.md)。

## ファイル命名

- `QA-<topic>-<YYYYMMDD|Issue番号>.md`（例: `QA-late-money-20260714.md` / `QA-analyze-384.md`）

## 書き方

各質問は「問い / 現状の観測・根拠 / 回答（確定 or 保留）/ 反映先」を持つ。

```markdown
## Q1: <問い>
- 観測/根拠: <コード・データ・ADR 等の裏付け>
- 回答: <確定した答え。未確定なら「保留（理由）」>
- 反映先: docs/specifications/<file>.md / docs/knowledge/<file>.md / ADR 起票
```

- 回答が確定したら、対応する knowledge に差分反映し、その knowledge の `updated`/`変更履歴`/
  `distilled_from_sha` を更新する。
- 矛盾が出たら反映先 knowledge を `status: Conflict` にして解消する。
- **qa 自体は生ファイル**。運用ルールや確定知を qa に書き残さない（それは knowledge/ADR の役割）。
