# 蒸留ガイド（paddock 固有）

HVE の KnowledgeManager が担う蒸留を、paddock の Claude Code セッションで実行するためのルール。
規約の正本は [knowledge/README.md](../../../../knowledge/README.md)。

## 蒸留の原則

1. **差分マージ**: 全書き換えしない。変更箇所のみ更新する（冪等）
2. **出典必須**: 推論で補完した箇所は `TBD（推論: {根拠}）` と明記する
3. **捏造禁止**: source に無い事実を書かない。source が曖昧なら `Tentative` にする
4. **最小差分**: 蒸留に関係ない文体修正・リフォーマットをしない

## frontmatter の更新手順

```yaml
distilled_from_sha: "<short-sha>"   # scripts/bump-distilled-sha.py <file> で更新
updated: "YYYY-MM-DD"               # 本文が実質変わった時だけ手動で進める
```

- `distilled_from_sha` は `bump-distilled-sha.py` に任せる（手書きしない）
- `updated` は sha bump だけの場合は触らない（「内容を実質更新した日」の定義）
- 同一コミットに自分の sha は書けないので、「本文コミット → sha 追従コミット」の 2 コミットになる

## REQ テーブルの扱い

paddock の REQ テーブルは 5 列固定:

```markdown
<!-- REQ:begin D{NN} -->
| REQ-ID | 要件 | 検証手段 | 出典 | status |
|---|---|---|---|---|
| REQ-D{NN}-001 | ... | ... | [...](path) | Confirmed |
<!-- REQ:end D{NN} -->
```

蒸留時のルール:
- 新しい要件 → 既存の最大番号 +1 で採番（番号空間はクラス内グローバル）
- 要件の廃止 → 行を消さず `status: Retired` にする（番号再利用禁止）
- `出典` 列に docs-original を名指しするなら、frontmatter の `sources` にも載せる（ADR 0083）
- `検証手段` が空なら `Confirmed` にできない

## D クラス別の蒸留注意点

### D22: 予測モデル

- 本番構成の定数（α, m, 冪較正, trend_n, 各 factor の重み）は REQ 表で管理
- backtest の実測値を根拠にする場合は、backtest の条件（`--shrinkage-m 10` 等）を明記
- 棄却した factor は D24 へ移す（D22 には「採用されたもの」だけ残す）

### D23: 買い方

- CLAUDE.md の「買い方ルール」セクションと矛盾しないこと（CLAUDE.md が運用ルールの正本）
- ROI ゲート・軸ロック・提示形式は D01 が正本（D23 は券種構成・配分の具体）
- 棄却済み改善案は betting-rule-history.md に記録済み。再提案しない

### D24: 実験・棄却証跡

- 棄却した実験は「なぜ棄却したか」の根拠を残す
- backtest の N（レース数）と期間を明記する
- 将来の再検証条件があれば書く（「データが N 件を超えたら再検証」等）

## 横断検索の活用

蒸留前に mdq で関連チャンクを検索すると、影響範囲を把握しやすい:

```sh
scripts/mdq search --q "<source の主要キーワード>" --top-k 10
```

同じテーマを扱う knowledge が複数ある場合、重複を避けて主たる文書に集約する。
