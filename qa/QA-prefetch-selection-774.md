# QA: prefetch のレース選択失敗の扱い（#774・2026-10-12）

`prefetch_odds.sh` が 2026-08〜09 にレース選択で失敗し続けていた件の確認。
一次資料: `docs-original/774-prefetch-selection-failures.md`。蒸留先: `knowledge/netkeiba-datasource.md`（決定ログ #774）。

## Q1: 原因を特定して直せるか

- 観測/根拠: 2026-08 は同じ launchd の PATH の python3 に `zoneinfo` が無かった（snapshot-coverage の traceback 449 回）。
  選択も同じ import をするので同じ理由で落ちていたとみられる。2026-09 は選択の stderr が `/tmp` で消えていて確定できない。
  2026-10-02 に Homebrew の python3 が入れ直され、調査時点では launchd と同じ条件で選択が通る。
- 回答: **当時の環境はもう残っていないので、根本原因は直せない**。8 月は zoneinfo の無い python3、9 月は不明として記録する。
  代わりに、次に壊れたら最初の 1 回で理由が本体ログに残るようにする。
- 反映先: knowledge/netkeiba-datasource.md（決定ログ #774）

## Q2: 診断をどこまで足すか

- 選択肢:
  - **選択の stderr を本体ログに写し、失敗時に使った python3 のパスと版を 1 行残す**（推奨）
  - 上に加えて、launchd の `StandardOutPath` / `StandardErrorPath` を `/tmp` から `~/Library/Logs` に移す
  - 上に加えて、launchd で使う python3 を固定する（版を明示する・起動時に import を自己診断する）
- 回答: **stderr の転記と python3 の記録の 2 点**。stderr を本体ログへ写すのは失敗のときだけにする
  （成功時の件数行を 5 分おきに積まない）。
- 反映先: knowledge/netkeiba-datasource.md（本文「エラー・欠損ハンドリング」・決定ログ #774）
