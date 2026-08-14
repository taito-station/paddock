# QA — `sources` の網羅性検査（#596 / #597）

一次資料: [docs/original-docs/0083-sources-coverage-checks.md](../original-docs/0083-sources-coverage-checks.md)

親 issue: [#579](https://github.com/taito-station/paddock/issues/579)（転記しない・ADR 0074）。
本文は `gh issue view 596` / `gh issue view 597` で取得する。

## Q0: そもそも現状に違反はあるのか（検査を書く前の実測）

- 観測/根拠: main `5ae6466` 時点で、`check-doc-classes.py` のパーサ
  （`parse_frontmatter` / `parse_req_blocks` / `RE_MD_LINK`）をそのまま流用して数えた。

  | 検査対象 | 母数 | 違反 |
  |---|---|---|
  | ADR の被参照（どこかの `sources` にあるか） | ADR 80 本 | **1 本** — `0074-no-issue-body-transcription.md` |
  | REQ 表 `出典` ⊆ 同文書の `sources` | REQ 行 26 / 出典のリポ内リンク 37（ユニーク 29） | **0 件** |

  出典セルのリポ内リンクは **37 本すべてが `docs/original-docs/` の 4 桁 ADR**（非 ADR は 0 本）。
  外部 URL は 1 本（REQ-D22-008 の GitHub issue）。
- 回答: **確定。`sources` を補う作業は発生しない。** 唯一の違反は issue が事前に予告していた
  ADR 0074 で、これは例外として扱うべきもの。両検査とも導入コストを払わずに error 化できる。
- 反映先: ADR 0083「コンテキスト」の実測表

## Q1: orphan ADR の例外はどこで宣言するか

- 観測/根拠: 選択肢は (a) スクリプト内の定数リスト、(b) `docs/knowledge/doc-classes.md` の宣言表。
  doc-classes.md には既に `doc-classes-na`（N/A 宣言）と `doc-classes-index`（割当索引）という
  マーカー付き宣言表があり、`extract_block()` がそのまま使える＝**機構の追加はゼロ**。
- 回答: **確定。(b) doc-classes.md の宣言表。** 定数リストにすると例外を増やす行為が
  「Python の変更」になり、文書レビューの視界から外れる。宣言表なら例外の追加が
  文書差分として残り、理由もレビューされる。ADR 0073 / 0074 の「規律に委ねない」と一貫する。
- 反映先: ADR 0083 決定 3 / `docs/knowledge/doc-classes.md`

## Q2: 例外表のパス形式は割当索引に合わせるか、`sources` に合わせるか

- 観測/根拠: 割当索引は `docs/` を剥がした形式（`knowledge/glossary.md`）、`sources` は
  リポジトリルート相対（`docs/original-docs/0073-....md`）。#597 の issue も
  「唯一の罠は基準パスの違い」と指摘している。
- 回答: **確定。`sources` と同一形式（ルート相対）。** 例外表の比較相手は `sources` の和集合なので、
  正規化を 1 段挟むほど「どちらの形式か」の事故が増える。比較相手に合わせるのが最も読み違えにくい。
  表の直上にその理由を書き、割当索引との形式差が意図的であることを明示する。
- 反映先: ADR 0083 理由 / `docs/knowledge/doc-classes.md`

## Q3: error か warning か

- 観測/根拠: 既存 10 項目のうち warning は「充足ギャップ」「サブディレクトリの .md」「履歴を辿れず
  判定できなかった」「shallow clone で distilled_from_sha を解決できない」の 4 つだけで、いずれも
  **不変条件そのものではない**。#580 は stale を
  warning → error に昇格させており、その理由は「warning のままだと写した量に比例して
  追従漏れが静かに溜まる」。懸念は「ADR を先に置いて写しを後続 PR に回す運用が塞がる」点。
- 回答: **確定。両方 error。** 現状の違反が 0〜1 件（しかもその 1 件は既知の例外）なので、
  導入コストを払わずに error にできる。ADR 先置きが本当に必要なら**例外表に理由付きで登録する**のが
  正規の逃げ道で、そのほうが「なぜ写しが無いのか」が残る。`--warn-only` は逃げ道として
  数えない（CI では使わない）。
- 反映先: ADR 0083 決定 1・2 / `scripts/check-doc-classes.py`

## Q4: #597 の突合対象を「ADR」に限るか、`docs/original-docs/` 配下に広げるか

- 観測/根拠: issue #597 は「対象は ADR に限る」と書きつつ、その根拠として挙げているのは
  「GitHub issue の絶対 URL は一次資料ファイルではないのでスキップ」＝**ファイル/URL の区別**。
  実測では出典リンク 37 本すべてが 4 桁 ADR なので、どちらを採っても現状の判定は変わらない。
- 回答: **確定。`docs/original-docs/` 配下に広げる（4 桁 ADR に限定しない）。** issue の字面は
  「外部 URL を弾く」意図で書かれており、`sources` が watch すべきは一次資料層そのもの。
  issue 由来の一次資料（`382-...`）を対象外にすると、そこだけ穴が残る。一方で
  knowledge / specifications 同士の相互参照は**対象外**にする——蒸留元ではなく相互リンクなので、
  `sources` に載せる筋合いが無く、載せると stale が本題と無関係な理由で発火する。
- 反映先: ADR 0083 決定 2・却下案 / `check_req_blocks()`

## Q5: 例外表そのものが腐るのをどう防ぐか

- 観測/根拠: N/A 宣言表は「一覧で n/a なのに宣言表に理由が無い」「宣言表にあるのに一覧が n/a でない」を
  双方向で突合している（`main()` の `na_in_table` / `na_declared`）。例外表に同じ手当てが無いと、
  ADR が後から `sources` に載っても例外行が残り続け、次に本当の orphan が出たとき
  「例外表にあるから安心」と誤読される。
- 回答: **確定。例外表の健全性も error で検査する。** ①実在しない ADR を挙げている
  ②実際は参照されている ADR を挙げている（＝不要になった例外）③行の書式が崩れている、の 3 つ。
- 反映先: ADR 0083 決定 5 / `scripts/check-doc-classes.py`
  （1 巡目のレビューで実装は 7 条件に増えた: ①実在しない ②実在するが ADR でない
  ③実際は参照されている ④行の重複 ⑤理由の空欄 ⑥見出し行の列名違い ⑦行の書式崩れ）

## Q6: PR をどう割るか

- 観測/根拠: #596 と #597 はどちらも `scripts/check-doc-classes.py` の `main()` 周辺と
  `scripts/test-check-doc-classes.py` を触る。別 PR にすると docstring の検査項目リストで
  必ずコンフリクトする。issue 自身も「片方が入ればもう片方の実装は軽くなる」と書いている。
- 回答: **確定。1 PR・2 コミット（`Closes #596, #597`）。** トピックは
  「`sources` の網羅性を機械検査にする」の 1 つで、コミットを issue 単位に割ることで
  bisect 可能性は保つ。ADR も 1 本（0083）にまとめる——2 つの検査は同じ穴の両側なので、
  決定としては 1 つ。
- 反映先: 運用のみ（文書に残す確定知ではない）

## 補足: ADR 番号の採番衝突（実地の記録）

本作業中、**同じ採番衝突が 2 回起きた**。いずれも並走セッションが #606 の対応で ADR を
起票したため:

1. 着手時に採番した **0081** を相手が先にコミット（`0081-pin-only-diff-is-not-content-change.md`）
   → 本 ADR を 0082 へ振り直し
2. main 取り込み時に **0082** も取られていた（`0082-swagger-ui-vendored.md`）
   → 本 ADR を **0083** へ振り直し

`scripts/check-adr-numbers.sh next` は実行時点のスナップショットしか返さないので、
**並走セッションがあるときは (a) commit 直前と (b) main 取り込み後の 2 回、採番を確認する**
（pre-push の `check-adr-numbers.sh` が重複を検出するので取り違えは残らないが、
振り直しは本文・写し・qa・コード中の参照すべてに波及するので早く気づくほど安い）。
