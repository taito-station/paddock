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
逆に片親と同じ内容になったマージ（B）は `--cc` が列挙しないが、**`git log` の既定の単純化も
そのマージを列挙しない**（TREESAME な親を辿る）ので、走査がそこへ来ることが無い。

### 4. `status is None` の分岐は何なのか（マージとは無関係）

上の議論は「`git log` が返す集合」と「`path_status` が status を返す集合」が一致することに
懸かっているので、**`status is None` の分岐に到達するかを計装して数えた**。

| 対象 | `status is None` の到達回数 |
|---|---|
| 実リポジトリ全体（`sources` 110 パス・全履歴） | **0 回** |
| 回帰テスト 183 ケース | **1 回**（非正規形パスの fixture） |

**この分岐はマージとは関係が無い。** 原因は `path_status` が name-status の**終点一致**
（`parts[-1] == path`）しか見ないことで、**非マージのコミットで起きる**:

- **リネーム元としてしか現れないコミット**。`R100 <path> <新パス>` の終点は新パスなので
  一致が外れる。`git log -- <path>` はこのコミットを列挙するので走査はここへ来る
  （合成履歴で再現: `c1` が `P.md` を作り、`c2` が `P.md` → `Q.md` の純粋リネーム。
  `git log -- P.md` は `c2` を列挙し、`path_status(c2, "P.md")` は `(None, None)`。`c2` は親 1 つ）。
- **`sources` が非正規形**（`./docs/...`）のとき。pathspec は正規化して当たるが `git show` は
  正規形で出力する。回帰テストの 1 回はこれ（検査 4 が別に error にするので production では踏まない）。

**この `continue` は load-bearing。** `return sha` に変えると純粋リネーム地点を「内容変更」と
誤認して**偽の STALE** を出す。当初これを pin するテストが無かった（`return sha` 変異で
183 ケースが全通過した）ので、`test_rename_source_commit_is_skipped_not_attributed` を足した
——両側と解決を免除対象にして走査がリネーム地点まで届く履歴を組む必要がある。

**計測の教訓**: 最初の計測は「183 ケースでも 0 回」だったが誤りだった。テストは checker を
`subprocess.run(..., capture_output=True)` で起動するので、**stderr へ出す計装はサブプロセス内で
捕捉されて外に出てこない**。ファイルへ追記する計装（環境変数でパスを渡す）に変えて測り直した。

## 決定

1. **evil merge の検出に実装を足さない。** 現行の `git show --format= --name-status -M100%`
   で 2 親・octopus とも検出できている。
2. **`--cc` への依存を契約として回帰テストで固定する。**
   `test_evil_merge_is_detected_as_content_change` は、**両親の変更を免除対象（ピン更新のみ）で
   挟み、マージの解決だけが内容を変える**形にする。こうしないと、マージが不可視になっても
   親側の変更が STALE を出してしまい、テストが何も識別しない（実際に一度そう書き、
   `path_status` がマージで `(None, None)` を返す変異を注入しても緑のままだった）。
   対照群として `test_pin_only_merge_is_not_stale` を置き、exit 1 が「マージだから」ではなく
   「内容が変わったから」であることを分離する。**対照群は解決に第 3 の hex を書く**——
   片親の hex をそのまま採ると対象パスについてその親と TREESAME になり、`git log` が
   マージを列挙せず `path_status` も免除分岐も一度も呼ばれない空テストになる。

   **壊し方は 2 種類あり、落ちるテストが違う**（実測。混同すると誤った安心を生む）:

   | 変更 | 出力 | 落ちるテスト |
   |---|---|---|
   | `git diff-tree`（`-c` 無し）/ `--diff-merges=off` | **マージが無出力** | `test_evil_merge_is_detected_as_content_change` |
   | `--first-parent` / `-m`（`git show` 側） | 第 1 親との差分が出る（**無出力ではない**） | `test_rename_inside_merge_is_treated_as_content_change` |
   | `--first-parent`（`git log` 側） | — | `test_merge_taking_one_side_is_attributed_to_ancestor` |
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

- **変更**: `scripts/test-check-doc-classes.py` に回帰テスト 5 件を追加
  （evil merge の検出 / その対照群 / 片親採用の祖先帰属 / マージ内リネームの偽 STALE /
  リネーム元としてしか現れないコミットを飛ばすこと）。
- **変更**: `scripts/check-doc-classes.py` の `path_status` docstring と
  `scan_last_content_change` の `status is None` 分岐のコメントを実測に合わせて訂正。
  **挙動は一切変えていない。**
- **不変**: 検査項目・severity・`--warn-only` の扱い・`scripts/bump-distilled-sha.py` が
  パースする STALE 行の文言。
- **運用**: `path_status` の `git show` 呼び出しにフラグを足すときは、決定 2 の表で
  **どのテストが落ちるはずか**を先に確かめる。`test_evil_merge_...` が落ちたなら穴が開いており、
  `test_rename_inside_merge_...` が落ちたなら「既知の偽陽性がたまたま消えた」ように見えるが
  実際には対象集合が第 1 親比較へ変わって別の偽 STALE が生まれている——**反転させてよい合図
  ではない**。
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
