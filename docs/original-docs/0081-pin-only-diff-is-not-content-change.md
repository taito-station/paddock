# 0081. `uses:` のピン留め SHA 更新だけの差分は「内容変更」と見なさない

## ステータス

承認済み（本 PR で実装）。対象 Issue: [#606](https://github.com/taito-station/paddock/issues/606)。
関連: [ADR 0073](0073-adr-into-original-docs-and-doc-classes.md)（機械検査の導入）/
[ADR 0077](0077-glossary-index-and-sources-scope.md)（`sources` の範囲。frontmatter を持たない
ファイルでは `is_metadata_only_change` が構造的に効かないことを記録済み）。

## コンテキスト

`.github/workflows/ci.yml` は [docs/knowledge/ci-pipeline.md](../knowledge/ci-pipeline.md) の
`sources` に入っている。ジョブ分割の設計意図を書いた文書なので、ワークフローが変わったら追従を
促すのは正しい。問題は**追従が必要な変更とそうでない変更を区別できていない**ことだった。

`scripts/check-doc-classes.py` の stale 検査は、`sources` の「最後に内容が変わったコミット」が
下流の `distilled_from_sha` の子孫なら error にする。除外機構は 2 つあるが、どちらも ci.yml には
効かない。

- 例外 1（`R100` リネーム）: パス移動ではないので対象外。
- 例外 1b（frontmatter のメタデータだけの変更）: `is_metadata_only_change` が
  `split_frontmatter` に依存しており、先頭行が `---` でない `.yml` は `new_fm is None` で
  即 `False` に落ちる。**ci.yml の変更は種類を問わず 100% 内容変更と判定される**。
  この構造的な穴は ADR 0077 が `CLAUDE.md` を `sources` に入れない理由として既に記録している。

結果として **ci.yml を触る PR はすべて `adr` ジョブで落ちる**。dependabot は自分のエコシステム外の
ファイル（下流 knowledge の frontmatter）を編集しないため、**Actions の SHA ピン更新 PR は構造的に
永久に赤**になる。

実害は観測済み。[#590](https://github.com/taito-station/paddock/pull/590)（dtolnay/rust-toolchain）と
[#591](https://github.com/taito-station/paddock/pull/591)（Swatinem/rust-cache）が 2026-08-10 から
`adr fail` のまま 2 日以上マージできず、さらに 2 本とも ci.yml を触るので片方をマージすると
もう片方が必ずコンフリクトする状態だった。最終的に人が
[#607](https://github.com/taito-station/paddock/pull/607) で 1 本に統合し、
「ci.yml コミット → `distilled_from_sha` 追従コミット」の 2 コミットを手で積んで解消した。

**SHA ピンはサプライチェーン対策なのに、その更新経路が機械的に塞がっているのは本末転倒**で、
セキュリティ更新の停滞に直結する。ピンは今後も定期的に上がるので、そのたびに人手が要る形は
「人手の規律に委ねない」という ADR 0073 の趣旨に反する。

## 決定

**`uses:` 行のピン留め SHA 更新だけの差分を、stale 検査の「内容変更」から外す**
（[docs/knowledge/README.md](../knowledge/README.md) の**例外 1d**）。

判定は `scripts/check-doc-classes.py` の `is_pin_only_change(sha, path)` が行い、
`last_content_change` が例外 1 / 1b と同じ位置で呼ぶ（該当コミットを飛ばして実質の変更点まで遡る）。
真になる条件は次のすべて。

1. 変更前後で**行数が同じ**（行の増減はジョブ・ステップの追加削除なので内容変更）
2. 変更のある行が **1 行以上**あり、その**すべて**が
   `^(\s*(?:-\s+)?uses:\s+)([^@\s]+)@([0-9a-fA-F]{40})(\s*#.*)?$` にマッチする
3. その各行で、**インデントと `uses:`（group 1）と owner/repo（group 2）が変更前後で同一**

変わってよいのは **40 hex（group 3）と末尾のバージョン注記（group 4）だけ**。末尾注記を許すのは、
dependabot が hex と一緒に `# v2` → `# v2.9.2` も書き換えるため（#607 の `a5cfa46` で実測）。
ここを許さないと `Swatinem/rust-cache` 系を取りこぼして例外が機能しない。

## 理由

**ピンの hex が上がっても、下流 knowledge の本文が語る内容は変わらないから。**
`ci-pipeline.md` が書いているのはジョブ構成と分割の設計意図であって、各 action の版ではない。
下流が読み直す理由が無い変更で追従を強制すると、追従は中身を見ない儀式に落ちる
（[#604](https://github.com/taito-station/paddock/issues/604) 要件 (e) が測ろうとしている劣化そのもの）。

**owner/repo の同一性を条件に入れるのは、action の差し替えはジョブの意味が変わるから。**
`dtolnay/rust-toolchain` を別の toolchain action へ替えるのは設計変更で、`ci-pipeline.md` の
記述が古くなりうる。hex とバージョン注記だけを許す形にすれば、ピン更新は通り、差し替えは止まる。

**タグへの緩和（`@<40hex>` → `@v4`）は片側が正規表現に合わないので自動的に内容変更になる。**
これは意図した挙動で、サプライチェーン対策の後退は下流に伝えるべき信号。

**例外 1d は「機械が吸収する」側の例外**（例外 1 / 1b と同列）で、人が bump する例外 1c とは
性質が違う。ここに置くことで、dependabot の素の PR が人手ゼロで通る。

## 却下した代替案

- **dependabot に `distilled_from_sha` を触らせる**: dependabot は自分のエコシステム
  （ここでは `.github/workflows/`）の外にあるファイルを編集しない。実現手段が無い。
- **人が拾う運用と割り切り、手順を文書化する**: 実装コストはゼロだが、#607 でやったことを
  Actions 更新のたびに再演する。ADR 0073 の「人手の規律に委ねない」に反し、
  ピンの更新頻度（dependabot が定期的に上げる）を考えると恒久的な人件費になる。
- **`ci.yml` を `ci-pipeline.md` の `sources` から外す**: 構造的な赤は消えるが、
  ジョブ構成が変わったときの追従も一緒に消える。`ci-pipeline.md` は「主題そのものが対象ファイル」
  なので `sources` に入れる判断は ADR 0077 で維持済み。捨てるべきは検査そのものではなく粒度の粗さ。
- **`is_metadata_only_change` を汎用化して非 Markdown も扱う**: `.yml` に「メタデータ」の
  一般的な定義は無い。ピン行という具体形に限った述語のほうが、例外が広がる余地が小さい。
- **hex のみの変更に限定する（末尾注記を許さない）**: dependabot が注記も書き換えるため、
  実際の PR が例外に乗らず問題が解決しない（#607 の実データで確認）。

## 影響

- dependabot の Actions 更新 PR が人手ゼロで `adr` ジョブを通る。ピン更新と説明コメントの改訂が
  同居する PR（#607 の形）は従来どおり stale になる——これは意図どおりで、
  `scripts/test-check-doc-classes.py` に対照ケースとして固定した（新規 6 ケース）。
- 例外が広すぎないことの担保はテストに依存する。ピン行以外の差分が 1 行でも混ざれば
  内容変更に落ちるため、「ci.yml を触った PR は何でも通る」への退行はテストで検出される。
- 行末の改行コードだけが変わった（CRLF ⇄ LF）コミットは、差分行 0 件となり
  `changed > 0` を満たさないので内容変更として扱われる（保守的側に倒す）。
- 同型の構造は `docs/api/openapi.json` を `sources` に持つ specifications 3 本
  （`prediction-search-api.md` / `rest-api-read.md` / `session-write-api.md`）にもあるが、
  生成物なので「実質変更なし」の差分が起きにくく、実害が観測されていない。本 ADR の対象外。
- `sources` にコードやワークフローを入れている文書は現状 ci-pipeline.md のみ。
  今後増えるときは、その種類ごとに「内容変更でない差分」の定義が要るかを検討する。
