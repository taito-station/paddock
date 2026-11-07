# hve-akm の paddock 固有の補足

paddock では hve-akm ではなく `/akm`（`.claude/skills/akm/SKILL.md`）を使う。hve-akm を使う場合は次に従う。

## hve-akm の「Step 1: Stale 検出」を上書き

`python3 scripts/check-doc-classes.py` で検査する（`.claude/scripts/hve/check-knowledge.py` は使わない。frontmatter の違いは knowledge-maturity の hve-local 参照）。一括追従は `python3 scripts/bump-distilled-sha.py --all-stale`。

## hve-akm の「Step 2: 蒸留（差分マージ）」を補足

- sha の追従は `python3 scripts/bump-distilled-sha.py <文書>` で行う（`.claude/scripts/hve/` のものは使わない）
- 決定を伴う変更は、該当する knowledge ファイルの `## 決定ログ` 節に新エントリを append する（#652）
- `updated` は下流の本文が実質変わったときだけ当日日付に進める（sha だけの追従では触らない）

## hve-akm の「Step 3: 横断整合性レビュー」を補足

検査は `python3 scripts/check-doc-classes.py` と `python3 scripts/check-decision-log-immutability.py` で行う（skill のチェックリストの `.claude/scripts/hve/` のコマンドの代わり）。frontmatter の必須項目は knowledge-maturity の hve-local に従う（`title` は不要、`distilled_from_sha` は文書ごとに単一の短縮 SHA）。
