# artifact-management の paddock 固有の補足

## artifact-management の「配置」を補足

paddock は**インライン方式**を採る（#652）。決定は、その決定が効く `knowledge/` 文書の末尾の `## 決定ログ` 節へ直接 append する。独立した ADR ファイル（`knowledge/adr/`）は作らない。旧 ADR（`0001-...`〜`0090-...`）は #652 で各 knowledge の決定ログへ移行済み・削除済み。

## artifact-management の「ID とステータス語」を上書き

- 見出しは `### ADR NNNN: 要約 (YYYY-MM-DD) — ステータス`（旧 ADR 由来）または `### #NNN: 要約 (YYYY-MM-DD) — ステータス`（新規の決定。issue 番号で採番）
- 新規の決定は `#NNN`（issue 番号）で書く。`ADR NNNN` の新規採番はしない
- 旧 ADR 番号の参照は `git grep 'ADR 0055'` で決定ログ見出しに当たる

## artifact-management の「機械検査」を上書き

paddock は `.claude/scripts/hve/check-decision-log.py` ではなく `scripts/check-decision-log-immutability.py` で検査する。既存エントリの改変・削除（誤字修正を含む）は error。pre-push hook と CI で自動実行される。
