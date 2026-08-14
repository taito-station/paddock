# 0084. evil merge は stale 検査から見えている（ADR 0081 の「既知の限界 (1)」の訂正）

## ステータス

承認済み。[#615](https://github.com/taito-station/paddock/issues/615) (a)。
**ADR 0081 の「既知の限界 (1)」を訂正する**（決定そのものは覆さない。0081 の例外 1d は有効）。

## コンテキスト

ADR 0081（ピン更新のみを stale 例外にする）のセルフレビューで、

> マージコミット自身だけが内容を変える evil merge は `git show` が既定でマージの差分を
> 出さないため恒久的に不可視（既存の挙動）

という指摘が出て、ADR 0081 の「既知の限界 (1)」として記録された。`scripts/check-doc-classes.py`
の `scan_last_content_change` にも同趣旨のコメントが置かれていた。#615 (a) はこれを消化する。

**指摘どおりなら fail-open**（`sources` に挙げたファイルが evil merge で書き換えられても
stale 検査が永久に気づかない）なので、まず実際に起こりうるか・本当に不可視かを実測した。

## 実測

### 1. 本リポジトリの履歴（全 369 マージ × `sources` 110 パス）

`git log -- <path>` が列挙したマージ × `sources` パスは **7 組**。そのすべてを `path_status` が
検出した（`MM` / `MA` / `AA`）。**不可視は 0 組**。

evil merge は現に起きている——例えば `8ec61a18`（#613 の main 取り込み）は、コンフリクトを
手で解決して**どちらの親にも無い内容**を作っており、`docs/qa/QA-sources-coverage-checks-596.md`
が `MA` で検出されて `last_content_change` はそのマージを返した。PR ブランチが main を
取り込んでコンフリクトを解消する運用がある以上、evil merge は日常的に発生する。

### 2. 合成 fixture（使い捨て git リポジトリ）

| # | 構成 | `path_status` | `last_content_change` |
|---|---|---|---|
| A | 真の evil merge（両親と異なる内容をマージで作る） | `MM` | **マージ自身** |
| B | 片親の内容をそのまま採るマージ | `None` | 祖先コミット（正しい） |
| C | マージ内での純粋なリネーム（内容差分ゼロ） | `RR` | マージ自身（＝偽 STALE） |
| D | octopus merge（3 親）での evil merge | `MMM` | **マージ自身** |

### 3. 機構

`git show` はマージに対して**既定で combined diff（`--cc`）**を出し、`--cc` は
**「全ての親と異なるパス」だけ**を列挙する。これは evil merge の定義そのもの。
逆に片親と同じ内容になったマージ（B）は列挙されないが、その内容を作ったコミットが
**祖先に実在する**ので飛ばしてよく、`git log` の TREESAME 単純化も同じ基準でそちらを辿る。
**両者の対象集合が一致するので穴が無い。**

## 決定

1. **evil merge の検出に実装を足さない。** 現行の `git show --format= --name-status -M100%`
   で 2 親・octopus とも検出できている。
2. **`--cc` への依存を契約として回帰テストで固定する。**
   `test_evil_merge_is_detected_as_content_change` は、**両親の変更を免除対象（ピン更新のみ）で
   挟み、マージの解決だけが内容を変える**形にする。こうしないと、マージが不可視になっても
   親側の変更が STALE を出してしまい、テストが何も識別しない（実際に一度そう書き、
   `path_status` がマージで `(None, None)` を返す変異を注入しても緑のままだった）。
   対照群として `test_pin_only_merge_is_not_stale` を置き、exit 1 が「マージだから」ではなく
   「内容が変わったから」であることを分離する。
3. **マージ内リネームの偽 STALE（実測 C）は塞がず記録する。** combined diff はリネームを
   `RR` として出す（`R100` ではない）ので免除分岐に当たらず、リネーム元も取れない。
   `test_rename_inside_merge_is_treated_as_content_change` で**現状の挙動として** pin する。
4. **ADR 0081 の「既知の限界 (1)」は誤りとして訂正する。** ADR は不変なので 0081 の本文は
   書き換えず、本 ADR と `docs/knowledge/ci-pipeline.md` の写しが正になる。
   0081 の「既知の限界 (2)」（CRLF で例外 1d が効かない）は**有効なまま**。

## 理由

- **fail-open の疑いは実測で否定された。** 「起こりうるか」は Yes（自分たちで日常的に作っている）
  だが、「不可視か」は No。指摘の前提だけが誤っていた。
- **誤った限界記述を残すコストが高い。** 実装コメントと ADR の両方が「ここは不可視」と
  書いていると、次に読む人が (a) 塞ぐ必要のない穴を塞ぐ実装を足す、(b) 実際には検査されている
  経路を「どうせ見えない」と扱う、のどちらかをやる。#615 が起票されたこと自体がその実例。
- **契約テストが無いと、この性質は簡単に失われる。** `--cc` は `git show` の既定であって
  明示的に書かれていないので、`--first-parent` の追加や `git diff-tree` への置換で黙って消える。
  実装ではなくテストで守るのが正しい（挙動を変えずに退行だけ捕まえられる）。
- **リネームの偽陽性を塞がないのは fail-closed だから。** 偽の STALE は「差分マージして
  sha を更新する」で解消でき、見落としと違って気づけない害にならない。発火実績も無い。

## 却下した代替案

- **`-m` / `--first-parent` を併用してマージを常に第 1 親と比較する。** 対象集合が変わり、
  **片親と同じ内容になったマージ（B）まで「内容変更」に化けて偽 STALE を量産する**。
  実測でも、この変更を入れると `test_rename_inside_merge_is_treated_as_content_change` が
  落ちた（リネームが `R100` に見えるようになり免除が効いてしまう）——つまり挙動が広範に変わる。
  検出できていない穴を塞ぐための変更ではないので、得るものが無い。
- **マージ内リネームの偽陽性を塞ぐ。** 各親に対して `-M100%` 付きの diff を取り直し、
  すべての親でリネームなら免除する、という実装は書ける。ただし発火実績が無く、
  fail-closed 側で、`path_status` の戻り値（単一 status）の形を変える必要がある。
  必要になってから入れる（`test_rename_inside_merge_is_treated_as_content_change` を
  反転させるのが正しい入口）。
- **ADR 0081 の本文を直接訂正する。** 「一度置いた ADR は改変しない」（CLAUDE.md）に反する。
  決定記録を後から書き換えると、当時何を根拠に決めたかが失われる。

## 影響

- **変更**: `scripts/test-check-doc-classes.py` に回帰テスト 4 件を追加
  （evil merge の検出 / その対照群 / 片親採用の祖先帰属 / マージ内リネームの偽 STALE）。
- **変更**: `scripts/check-doc-classes.py` の `path_status` docstring と
  `scan_last_content_change` の `status is None` 分岐のコメントを実測に合わせて訂正。
  **挙動は一切変えていない。**
- **不変**: 検査項目・severity・`--warn-only` の扱い・`scripts/bump-distilled-sha.py` が
  パースする STALE 行の文言。
- **運用**: `path_status` の `git show` 呼び出しにフラグを足すときは、上記の契約テストが
  落ちないかを見る。落ちるなら stale 検査に穴が開いている。
- 関連: ADR 0081（例外 1d と `ScanAborted` の error 昇格）/ ADR 0073（機械検査の導入）/
  ADR 0083（`sources` の網羅性検査）/ #612 / #615。

## 再現方法

```sh
# 契約が守られていること
python3 scripts/test-check-doc-classes.py

# 契約テストが本当に効くこと（変異テスト）:
# path_status の先頭に「マージなら (None, None) を返す」を差し込むと
# test_evil_merge_is_detected_as_content_change が落ちる

# 本リポジトリでの実測（マージ × sources パスのうち不可視が 0 件であること）
# … git log -- <path> が列挙したマージに path_status を当てて数える
```
