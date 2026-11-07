# knowledge-maturity の paddock 固有の補足

## knowledge-maturity の「frontmatter 標準」を上書き

paddock の frontmatter は `knowledge/README.md` が正。標準との違い:

- `title` は持たない
- `distilled_from_sha` は文書ごとに単一の短縮 SHA
- `doc_class` と mdq 用ミラーの `tags` を持つ（定義は `knowledge/doc-classes.md`）

`updated` の規則（sha だけの追従では進めない）は標準の「`updated` の規則」に従う。

## knowledge-maturity の「sha の追従」を補足

追従は `.claude/scripts/hve/bump-distilled-sha.py` ではなく `scripts/bump-distilled-sha.py`（文書指定・`--all-stale`・`--dry-run`・`--sha`）で行う。`--follow-rewritten` は無いので、squash・rebase の後に sha が HEAD から外れた文書は追従させるファイル名を明示して bump する。

## knowledge-maturity の「検査スクリプト」を上書き

paddock は `.claude/scripts/hve/` を使わず、リポジトリの `scripts/` で検査する（CI と pre-push で実行）。

| スクリプト | 役割 |
|---|---|
| `scripts/check-doc-classes.py` | stale 判定（sources が `distilled_from_sha` より後に更新されたか）、本文の相対リンク、`doc-classes.md` との突合、REQ 表の出典 |
| `scripts/bump-distilled-sha.py --all-stale` | stale な文書の `distilled_from_sha` を一括追従（`updated` は触らない） |
| `scripts/check-decision-log-immutability.py` | 決定ログの append-only 性 |

## knowledge-maturity の「組み込み方」を上書き

上の `scripts/` のスクリプトを CI と pre-push に組み込み済み。`.claude/scripts/hve/` は setup.sh の配布物として置くだけで、配線しない。
