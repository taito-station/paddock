# QA: 週次の Security Audit の修正（#820・2026-11-08）

`.github/workflows/audit.yml` の週次監査が 2026-08-31 から毎回 failure になっていた件で、着手前に確定した点。
回答は 2026-11-08 の会話でユーザーが選んだもの（AskUserQuestion）。
蒸留先: `knowledge/ci-pipeline.md`（決定ログ #820）。

## 着手前に確かめた事実

- 検出している脆弱性（2026-11-08。ローカルの cargo-audit 0.22.2 で `cargo audit`、`npm --prefix web audit` で計測）:
  - cargo: 20 件。Issue の 15 件は CI が最後に動いた時点の数で、その後 advisory DB が更新された。このほか unmaintained 3 件・unsound 13 件の警告がある
  - npm: 14 件（moderate 10・high 4）
- cargo-audit の公式リリース（rustsec/rustsec の `cargo-audit/v0.22.2`・2026-06-05）には、Linux の gnu 版と musl 版の tgz がある。
  musl 版 `cargo-audit-x86_64-unknown-linux-musl-v0.22.2.tgz` の sha256 は `7fb9497f8594b389e5fce5ef9b92db08432996895b2e0c5a0167a69ed445c428`。
  手元で取得して計った値と、GitHub API の asset の `digest` が一致した。tgz の中身は `cargo-audit-x86_64-unknown-linux-musl-v0.22.2/cargo-audit`
- gnu 版のバイナリは GLIBC_2.39 を要求する（計画のレビューで確認）。musl 版は静的リンクでこの依存が無い
- checkout のピン `3d3c42e5aac5ba805825da76410c181273ba90b1` は `refs/tags/v7.0.1`（と `v7`）を指し、コメントの `# v4` は誤り

## Q1: cargo-audit をどう入れるか

- 選択肢:
  - 公式バイナリを sha256 で固定する
  - `cargo install --locked --version` で入れる
  - audit-check を残して `issues: write` を足す
  - 監査と報告のジョブを分ける
- 回答: **公式バイナリを sha256 で固定する**。第三者のコードをビルドせず、書き込み権限も要らない。
- 計画のレビューを受けて、gnu 版ではなく musl 版を使う（ランナーの glibc に依存しないため）。

## Q2: 検出している脆弱性をどう扱うか

- 選択肢:
  - 別 Issue にして、監査は red のまま残す
  - この PR で依存も上げる
  - 理由を付けて ignore する
- 回答: **別 Issue にして、監査は red のまま残す**。1 PR = 1 トピックにするため。ignore すると、修正版があるものまで隠れる。
  起票した Issue は #825（cargo）と #826（npm）。

## Q3: checkout のピンのコメント `# v4` を直すか

- 回答: **`audit.yml` と `ci.yml` の両方を直す**（`# v7.0.1`）。dependabot の PR をレビューするときに、版を読み違えないようにするため。

## Q4（PR のセルフレビュー中）: npm-audit ジョブの `npm ci` を外すか

- 背景: `npm audit` は `package-lock.json` だけで動く（node_modules が無くても同じ 14 件を検出することを確かめた）。それなのに `npm ci` が、第三者の lifecycle scripts を毎週動かしていた。
  Analyst は「#820 の成功条件（書き込み権限のあるジョブで動かさない）は満たしており、範囲外」と裁定した。
- 回答: **この PR で外す**（`npm ci` と `setup-node` の `cache: npm`）。監査ジョブから第三者のコードの実行をなくす、という #820 の趣旨にそろえるため。`ci.yml` の `npm ci` はビルドに要るので触らない。

## Q5（PR のセルフレビュー中）: cargo-audit を取得する curl にリトライを付けるか

- 回答: **付ける**（`--retry 3 --retry-all-errors`）。一時的な障害による red が、脆弱性による red に紛れないようにするため。取得物は毎回 sha256 で照合するので、安全性は変わらない。
