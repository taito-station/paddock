# QA: dependabot の cooldown（#817・2026-11-07）

#728 の残るリスク（ハッシュは乗っ取られたメンテナが出した新しい版を防がない）への対策として、dependabot の 4 entry（cargo・npm・github-actions・pip）に `cooldown` を入れるにあたり、着手前に確定した点。
回答は 2026-11-07 の会話でユーザーが選んだもの（AskUserQuestion）。
蒸留先: `knowledge/ci-pipeline.md`（決定ログ #817）。

## 着手前に公式 Docs で確かめた事実

出典: GitHub Docs「Dependabot options reference」の `cooldown` と「Optimizing pull request creation for version updates」の「Setting up a cooldown period」（2026-11-07 に参照）。

- `cooldown` を書かなくても、version updates には既定で 3 日の cooldown がかかる。#817 の起票時に書いた「既存の entry のどれにも入っていない」は、明示の設定が無いという意味で、実際の挙動は 3 日だった。
- `default-days` は全エコシステムで使える。semver 別の日数（`semver-major-days` などの `semver-*-days`）は github-actions では使えない（cargo・npm・pip は使える）。
- cooldown は version updates だけに効き、security updates には効かない。Issue で「未確認」としていた点はこれで解消した。
- 日数は 1〜90 日（「Optimizing pull request creation」の「The number of cooldown days must be between 1 and 90.」）。
- Docs は「新しい版が cooldown 中の依存は更新をスキップする」と依存ごとの扱いを書くだけで、次の 3 点は明記していない（未確認）。
  - groups（minor/patch のグループ PR）の中での扱い。cooldown 中の依存だけがグループから外れるのか。
  - cooldown を過ぎた旧い版を代わりに提案するか（7 日より短い間隔で版を出し続ける依存が、提案され続けないことがあるか）。
  - cargo・npm で lockfile を作り直すときに一緒に動く推移的な依存に、cooldown が効くか。

## Q1: cooldown の日数をどうするか

- 選択肢:
  - 一律 7 日
  - 7 日に加えて major だけ 14 日
  - 一律 14 日
  - 既定の 3 日を明示する
- 回答: **4 entry すべてに一律で `default-days: 7`**。
  - github-actions は semver 別の日数に対応していないので、一律にすれば 4 entry が同じ書き方になる。
  - major はもともと個別 PR で人が見るので、major の日数を延ばしても効果は小さい。
  - 既定の 3 日を明示するだけでは、攻撃に対して稼げる時間が増えない。
- 遅れの見込み: cooldown は公開日から数え、schedule は毎週月曜なので、更新が提案されるのはおおむね公開から 7 日以上 14 日未満になる（既定の 3 日では 3 日以上 10 日未満）。
