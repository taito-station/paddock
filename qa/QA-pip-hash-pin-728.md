# QA: CI の pip 依存のハッシュ検証と更新監視（#728・2026-11-07）

#719・#720 で predict-check ジョブに足した pip install（PyPI から第三者パッケージを取る）の供給網対策で、着手前に確定した点。
回答は 2026-11-07 の会話でユーザーが選んだもの（AskUserQuestion）。
蒸留先: `knowledge/ci-pipeline.md`（決定ログ #728）。

## Q1: harness の依存（`scripts/harness/requirements-ci.txt`）も対象に含めるか

- 背景: Issue の起票後に #720 で、同じ predict-check ジョブの別ステップが `scripts/harness/requirements-ci.txt`
  （`-r ../predict-check/requirements-ci.txt` に scipy を足した完全ピン）を pip install するようになった。
  ファイル内のコメントも「ハッシュ検証は #728 で扱う」と書いている。
- 回答: **含める**。片方だけハッシュ検証にしても、もう片方のステップが numpy 一式をハッシュ無しで入れ直すので、リスクが残る。

## Q2: `actions/checkout` の `persist-credentials: false` をどこまで入れるか

- 背景: `actions/checkout`（ピンしている版は v7.0.1）は既定で GITHUB_TOKEN を `$RUNNER_TEMP` の資格情報ファイルに残し、
  `.git/config` から `includeIf` で参照させる。どのジョブも書き込みの git 操作はしない。
  `docker-build` ジョブの「Detect relevant changes」の `git fetch` は、公開リポジトリなので認証無しで通る。
- 回答: **全ジョブに入れる**（`ci.yml` と `audit.yml` の checkout すべて）。第三者コード（PyPI・npm・cargo の build script）から
  git の設定経由でトークンを読めないようにし、方針を 1 つにする（メモリ上のトークンは消せないので、権限の最小化は Q5）。

## Q3: pip のキャッシュ（`actions/cache`）を入れるか

- 回答: **入れない**。venv の作成・install・pytest を含む 2 ステップで合計約 46 秒（main の CI 実行 42348153658）で、速度の動機が無い。キャッシュは miss すれば PyPI に行くので、上流障害で必須チェックが
  落ちるリスクは消えない（ADR 0082 で rust-cache を根治にしなかったのと同じ）。残るリスクとして記録する。

## Q4（Plan のレビュー中）: Dependabot alerts を有効にするか

- 背景: pip-audit を入れない理由に「脆弱性は dependabot alerts が拾う」と書こうとしたが、このリポジトリは alerts が無効だった
  （`gh api repos/<owner>/<repo>/vulnerability-alerts` が 404・`dependabot_security_updates` が disabled）。
  pip に限らず cargo・npm の依存も脆弱性の通知が来ていなかった（週次の `audit.yml` は cargo・npm だけ）。
- 回答: **有効にする**（リポジトリ設定の変更で PR の外）。2026-11-07 に有効化した。security updates（自動の修正 PR）は無効のまま。

## Q5（PR のセルフレビュー中）: `audit.yml` の `checks: write` をどうするか

- 背景: `audit.yml` は workflow 全体に `checks: write` を付けていて、`npm ci`（第三者コードの lifecycle scripts）を動かす
  `npm-audit` ジョブにも書き込み権限付きのトークンが渡っていた（既存の状態）。`persist-credentials: false` ではメモリ上の
  トークンは守れない。
- 回答: **この PR で直す**。`checks: write` を `cargo-audit` ジョブ単位に移し、workflow 全体は `contents: read` だけにする。

## Q6（PR のセルフレビュー中）: dependabot の pip で scikit-learn を除くか

- 背景: `/scripts/harness` を監視すると、同じディレクトリの `requirements.txt`（棄却済み路線・更新しない）も対象に入りうる。
  scikit-learn はそこにしか無いので、更新 PR は必ず不要になる。
- 回答: **`ignore` に足す**。numpy・scipy の範囲を広げる PR は major の境界でしか来ないので、来たら閉じる運用のままにする。

## Q7（PR のセルフレビュー中）: dependabot の `cooldown` を入れるか

- 背景: ハッシュは既存の版のファイルが変わることは防ぐが、乗っ取られたメンテナが出した新しい版は防がない。公開直後の版の提案を遅らせる
  `cooldown` は既存の cargo・npm・github-actions にも無い。
- 回答: **別 Issue（#817）で全エコシステムにそろえて入れる**。この PR では残るリスクを決定ログに書くだけにする。

## 補足（PR のセルフレビュー 2 巡目で分かったこと）

- Q5 の判断の前提に誤りがあった。`cargo-audit` ジョブも、`rustsec/audit-check` が版を固定せずに `cargo install cargo-audit`
  を実行して約 300 クレートをビルドするので、第三者コードを動かす。`checks: write` の移動は、書き込み権限を渡すジョブを
  1 つに絞っただけになる。また `checks: write` が使われるのは `workflow_dispatch` のときだけで、schedule 起動は Issues API で
  報告しようとして権限不足で毎週落ちている（2026-08-31 以降・既存の不具合）。
- この PR では `checks: write` の移動を残し、文書を事実に合わせるだけにする（Analyst の裁定）。audit-check の置き換えと
  schedule の失敗は別 Issue で扱う。
