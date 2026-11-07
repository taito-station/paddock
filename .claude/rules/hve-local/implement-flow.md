# implement-flow の paddock 固有の補足

## implement-flow の「Step 2: Knowledge 検索（Plan の各領域を網羅）」を補足

- 確定知は `knowledge/` に一本化（ADR 0092）。決定は各 knowledge 末尾の `## 決定ログ` 節（独立した ADR ファイルは無い・#652）
- `docs/docs-generated/` は蒸留対象外
- 検索は `scripts/mdq search` / `scripts/cq search` を使う（ルート `CLAUDE.md` の探索規律が正）

## implement-flow の「Step 7: Knowledge 同期」を補足

- 運用手順・買い方ルールを変えたら、ルート `CLAUDE.md`（買い方は `knowledge/glossary.md` も）と該当 skill も同じ PR で直す
- 決定ログは各 `knowledge/` 文書の末尾の `## 決定ログ` 節に append する（artifact-management の hve-local 参照）
- 検証コマンド:
  - `python3 scripts/bump-distilled-sha.py --all-stale --dry-run`（stale 0 件を確認）
  - `python3 scripts/check-doc-classes.py`
  - `python3 scripts/check-decision-log-immutability.py`
