# 0082. `sources` の網羅性を機械検査にする（orphan ADR / REQ 出典の突合）

## ステータス

承認済み。ADR 0073 の後続（[#579](https://github.com/taito-station/paddock/issues/579) の段階的タスク・
[#596](https://github.com/taito-station/paddock/issues/596) / [#597](https://github.com/taito-station/paddock/issues/597)）。

## コンテキスト

ADR 0073 は「ADR の内容は knowledge へ全部写す。重複を許す代わりに同期切れは機械で検出する」を選び、
その担保として `scripts/check-doc-classes.py` の **stale 検査**（`sources` に挙げた一次資料が
`distilled_from_sha` より後に更新されていたら error）を置いた。

**この担保には網羅性の穴がある。** stale が見るのは「`sources` に**挙がっている**行」だけで、
`sources` の**中身が正しいか**は誰も見ていない。

- **`sources` から行を消せば stale も消える。** 追従が面倒な出典を削るだけで検査を黙らせられる。
- **ADR を足したときに `sources` へ載せ忘れても、何も起きない。** その ADR が後で更新されても
  下流は「追従すべき文書」として認識されないまま静かに古くなる。
- **REQ 表の `出典` 列が名指しした ADR も同じ。** 本文では根拠として挙げているのに `sources` に
  無ければ、その根拠が変わっても要件側は気づかない。

実測（main `5ae6466` 時点）では不変条件はまだ真だった。

| 検査対象 | 母数 | 違反 |
|---|---|---|
| ADR の被参照（どこかの `sources` にあるか） | ADR 80 本 | **1 本**（`0074-no-issue-body-transcription.md`） |
| REQ 表 `出典` 列 ⊆ 同文書の `sources` | REQ 行 26 / 出典のリポ内リンク 37（ユニーク 29） | **0 件** |

**真であるうちに検査を入れないと、次に ADR を足した誰かが静かに壊す。** これは ADR 0073 / 0074 が
繰り返し排除してきた「人手の規律に委ねる」構図そのもので、`app-bootstrap.md` の `NoopParser` 事故
（status が Confirmed のまま存在しない実装を推奨し続けた）と同型の経路。

## 決定

`scripts/check-doc-classes.py` に検査を 2 つ足し、例外は文書側で宣言する。

1. **orphan ADR 検査 [error]**（#596）。`docs/original-docs/` の **0 埋め 4 桁の ADR**（判定の
   述語は決定 4）を列挙し、
   全 knowledge / specifications の `sources` の和集合に含まれないものを error にする。
   issue 由来の一次資料（`382-...` のように 0 埋めしない番号）は蒸留先を持つとは限らないので対象外。

2. **REQ 出典 ⊆ sources 検査 [error]**（#597）。REQ 表の `出典` セルが名指しした
   **`docs/original-docs/` 配下のファイル**が、その文書の frontmatter の `sources` に無ければ error。
   GitHub issue の絶対 URL など**リポジトリ外の参照は対象外**（一次資料ファイルではないので
   `sources` に載せられない）。knowledge / specifications 同士の相互参照も対象外
   （蒸留元ではなく相互リンクなので、`sources` に載せる筋合いが無い）。
   基準パスは**リポジトリルート相対に正規化**して比較する（`出典` 列は文書からの相対、
   `sources` はルート相対で、片方に寄せないと必ず食い違う）。

3. **例外は `docs/knowledge/doc-classes.md` のマーカー付き宣言表で持つ。**
   `<!-- adr-orphan-exceptions:begin -->` … `:end` の 2 列表（`| ADR | 例外の理由 |`）に、
   **理由を必須**で書く。パスは `sources` と同一形式（リポジトリルート相対）にする。
   現時点の登録は **ADR 0074 の 1 本のみ**——文書運用の規約そのものを定めた ADR で、
   蒸留先の knowledge を持たない。

   **正当な例外カテゴリは 2 つ**: (a) 規約そのものを定めた ADR で写す先が無いもの、
   (b) **supersede されて下流が後継 ADR だけを参照するようになった ADR**。(b) は
   「決定を変えるときは新しい ADR で supersede する」（CLAUDE.md）と直交する必然で、
   下流が旧 ADR を `sources` から落とすのは正しい操作。落とした瞬間に orphan になるので、
   **supersede する PR で例外表に「ADR NNNN に supersede された」と書く**。

4. **ADR の判定は `scripts/check-adr-numbers.sh` と同一の述語（`^0[0-9]{3}`）にする。**
   先頭 0 を落とすと、issue 番号が 4 桁に届いた時点で issue 由来の一次資料
   （`1024-foo.md`）が ADR と誤判定され、「例外表に登録しろ」という誤った助言つきで
   CI が落ちる。judge を 2 本に割らない——本 ADR が塞いでいる second source と同型。

5. **例外表そのものも検査する。** 実在しない ADR／実在するが ADR ではないファイル／実際は
   `sources` から参照されている（＝不要になった例外）／行の重複／理由の空欄（決定 3 の裏返し）／
   見出し行の列名違い／行の書式崩れ、はいずれも error。N/A 宣言表と一覧の相互突合
   （ADR 0073 で入れた同型の検査）と同じ考え方で、**腐った例外を残さない**。

6. **`sources` と例外表のパスは正規形（`docs/...`）だけを許す。** `./docs/...` のような
   非正規形は実在検査を通るのに **stale 判定の突合から静かに外れる**（`path_status` が
   `git show --name-status` の出力と終点一致で突き合わせるため、`./` 付きは永久に一致せず
   「履歴を辿れず」の warning に退化する）。突合用に正規化した集合を別に持つ手もあるが、
   それは「どちらの形式か」を持つ場所を増やす——例外表のパス形式で却下したのと同じ理由で、
   **形式を 1 つに強制する**方を採る。導入時点で `sources` の全行・例外表 1 行すべて正規形
   （大文字小文字違いも同じ理由で error にする。macOS では実在検査を通り Linux の CI だけが
   落ちるうえ、手元では stale が warning に退化する）。

7. **ADR が 0 件なら error にする。** 判定条件やディレクトリの取り違えで検査 12 が丸ごと
   無言で無効化される fail-open を塞ぐ。これは「違反がある」ではなく**「検査が成立していない」**
   側の失敗なので、マーカー欠落と同じく **`--warn-only` でも抑止しない**。

## この検査が保証しないこと（意図的な限界）

**機械化できたのは「`sources` への登録」までで、「knowledge へ写したか」ではない。**
ADR を任意の文書の `sources` に 1 行足せば検査 12 は通る（実際
`docs/knowledge/product-goals.md` は 34 本の ADR を索引目的で `sources` に並べている）。
**写しの中身——決定・理由・却下案・影響を実際に書いたか——は人手の規律に残る。**
ADR 0073 決定 2 の担保としては部分的で、ここを誇張して書くと
「機械が見ているから大丈夫」という誤った安心を生む。

**参照元が複数ある ADR には、元の穴が開いたまま残る。** 導入時点で **ADR 81 本のうち 48 本（59%）が
2 文書以上から参照されている**（被参照数の分布 `{0:1, 1:32, 2:21, 3:15, 4:5, 5:2, 6:3, 7:1, 8:1}`）。
これらは 1 つの文書の `sources` から落としても検査 12 は鳴らず（他がまだ参照している）、その文書の
REQ `出典` に載っていなければ検査 11 も鳴らない。**塞げたのは「最後の 1 本を落とす」ケースと
「REQ が根拠として名指しした出典」だけ**で、中間——本文が根拠にしているが REQ 表の外にある参照——は
人手の規律に残る。

**検査 11 と 12 でスコープが非対称**なのも意図的。11 は `docs/original-docs/` 配下**全体**
（issue 由来の一次資料も蒸留元なので）、12 は **0 埋め 4 桁 ADR だけ**（issue 由来の一次資料は
調査所見の置き場で、蒸留先を持つとは限らない）。この結果、導入時点で非 ADR 一次資料 1 本
（`601-axis-flip-in-predict-watch.md`）と `docs/qa/` の 2 本（`QA-axis-lock-601.md` /
`QA-roi-gate-calibration-571.md`。各層の `README.md` は数えない）がどの `sources` からも
参照されていないが、これらは検査対象外なので鳴らない。

同様に、検査 11 は **`出典` セルの Markdown リンクしか見ない**。出典をプレーンテキストや
インラインコードで書けば突合されないので、**最も安い回避策は「出典のリンクを外す」**に
なった（塞いだ「`sources` から行を消す」より安い）。ここを error にしなかったのは、
既存の出典に `ADR 0001` のような素のテキスト表記と外部 URL のみの行が実在し、
一律必須にすると本題と無関係な修正を大量に強いるため。**残る穴として
`docs/knowledge/README.md` の「機械検査できない」リストに明記する**。

## 理由

- **stale 検査は `sources` が正しいことを前提にしている。** 前提の側を無検査にしたまま
  結論の側だけ機械化しても、抜け道が残っているぶん fail-open になる。片方だけ締めても意味が薄い。
- **error でなければ守れない。** #580 で stale を warning → error に昇格させたのは、
  「warning のままだと写した量に比例して追従漏れが静かに溜まる」を実測したため。同じ検査系に
  warning を混ぜると、その 1 項目だけが同じ経路で腐る。**現状の違反が 0〜1 件で、しかもその 1 件が
  既知の例外**なので、導入コストを払わずに error にできる。
- **例外はスクリプトでなく文書に置く。** スクリプト内の定数リストにすると、例外を増やす行為が
  「Python の変更」になり、文書レビューの視界から外れる。宣言表なら**例外を増やすたびに
  理由が文書差分として残り、レビューに乗る**。既存の `extract_block()` をそのまま再利用できるので
  機構の追加もゼロ。
- **例外のパスを `sources` と同形式にする。** 割当索引は `docs/` を剥がした形式だが、
  この表の比較相手は `sources` なので、正規化を 1 段挟むほど「どちらの形式か」の事故が増える。
  比較相手に合わせるのが最も読み違えにくい。
- **REQ の `出典` を選んだのは、そこが「根拠を名指しした」唯一の機械可読な場所だから。**
  本文の相対リンク全部を `sources` に強制すると、単なる相互リンクまで watch 対象になって
  `sources` がノイズで膨らむ。REQ 表の `出典` 列は定義上「その要件の根拠」なので、
  watch 対象であるべきという主張が成り立つ。

## 却下した代替案

- **例外をスクリプト内の定数リストで持つ**（`ORPHAN_EXCEPTIONS = {...}`）。実装は最小だが、
  例外の追加が文書レビューに乗らない。「規律に委ねない」という ADR 0073 / 0074 の一貫した
  方針に反する——例外こそレビューされるべきもの。
- **どちらか／両方を warning に留める。** 「ADR を先に置いて写しを後続 PR に回す」運用を
  塞がずに済むが、#580 が実証したとおり warning は無視され、機械検査の実効性が落ちる。
  ADR 先置きが本当に必要なら、**例外表に理由付きで登録する**のが正規の逃げ道で、
  そのほうが「なぜ写しが無いのか」が残る。
- **`sources` から行を消せない仕組み（append-only）にする。** 穴の原因そのものを潰せるが、
  出典が本当に不要になったとき（文書の分割・統合）に消せず、`sources` が単調増加する。
  網羅性は「消せない」ではなく「消したら本文と矛盾する」で守るほうが素直。
- **REQ の `出典` だけでなく本文の相対リンク全部を `sources` と突合する。** 網羅性は最大になるが、
  用語集や兄弟仕様への相互リンクまで `sources` に載せることになり、stale の発火が本題と
  無関係な理由で増える。`sources` の意味（蒸留元）が壊れる。
- **orphan の判定に `CLAUDE.md` からの参照も数える。** ADR 0077 で `CLAUDE.md` は `sources` に
  入らない設計にしたので、ここで参照元として数えると「`CLAUDE.md` に書いたから写しは不要」という
  抜け道ができる。ADR 0073 決定 2 が要求しているのは knowledge / specifications への写し。

## 影響

- **変更**: `scripts/check-doc-classes.py` に検査 11（REQ 出典 ⊆ sources）と 12（orphan ADR）を追加。
  docstring 冒頭の検査項目リストも更新する。
- **変更**: `docs/knowledge/doc-classes.md` に `adr-orphan-exceptions` マーカーブロックと
  本 ADR の写しを追加する。`scripts/test-check-doc-classes.py` の `REGISTRY_TEMPLATE` にも
  同じマーカーが要る（`extract_block()` はマーカー欠落で `sys.exit` する fail-closed のため）。
- **運用の変更**: **ADR を新設したら、同じ PR でどこかの knowledge / specifications の `sources` に
  載せる。** 載せられない ADR（規約そのものを定めた ADR / supersede 済み）は例外表に理由付きで
  登録する。**塞がるのは「`sources` への登録を後続 PR に回すこと」までで、本文の写しを後回しに
  する運用は依然として機械では止まらない**（上記「この検査が保証しないこと」）。
- **既存検査の厳格化**: 決定 6 は検査 4（`sources` の実在）に「非正規形・大文字小文字違いを
  error にする」分岐を足す＝**既存項目の挙動変更**（導入時点で違反 0）。また
  「サブディレクトリの `.md`」を warning から **error に昇格**させる——文書を 1 階層下げるだけで
  frontmatter 系・stale・REQ の一意台帳・`sources` の被参照が丸ごと外れ、警告 1 行のまま
  exit 0 になる（実測）。orphan 検査が入った今は他の ADR の誤判定にも波及するので、
  #580 が stale を昇格させたのと同じ理由で error にする（導入時点で該当 0 件）。
- **不変**: 上記以外の既存検査と `scripts/bump-distilled-sha.py` がパースする STALE 行の文言は
  変えない。`--warn-only` のセマンティクス（違反は警告扱いにして 0 で終了）も変えないが、
  **fail-closed（抑止できない）側に 2 つ加わる**——`adr-orphan-exceptions` マーカーの欠落
  （既存のマーカー欠落と同じ扱い）と、ADR 0 件（決定 7）。
- **並走 PR の競合**: ADR PR は必ず hub 文書の frontmatter（`sources` と `distilled_from_sha` は
  隣接する）を触るため、**同時期の ADR PR 同士はほぼ必ずコンフリクトする**。しかも競合を
  解決してマージしても、後発の `distilled_from_sha` が先発の ADR を含まないので **マージ後の
  main で STALE が出る**。branch protection の "Require branches to be up to date" を有効にするか、
  マージ後に `scripts/bump-distilled-sha.py --all-stale` を回す。
- **必ず 2 コミットになる**: 新設 ADR を `sources` に載せた時点で、その ADR の最終内容変更は
  現 `distilled_from_sha` より後になるので **必ず STALE が発火する**（自分の sha を同じコミットに
  書けないため）。本文コミットの後に `scripts/bump-distilled-sha.py --all-stale` で追従コミットを
  積む。本 ADR 自身もそうなっている。
- **副次的な効果**: 本 ADR 自身が orphan 検査の最初の対象になる（`doc-classes.md` の `sources` に
  載せることで満たす）。検査が自分自身に効いていることの実演になっている。
- 関連: ADR 0073（一次資料層への ADR 統合・stale 検査）/ ADR 0074（転記しない）/ ADR 0077
  （`CLAUDE.md` を `sources` に入れない）/ #579（親）/ #580（stale の error 昇格）/ #594 / #604。

## 再現方法

```sh
# 検査が通ること（false positive が無いこと）
scripts/check-doc-classes.py            # → exit 0

# orphan 検査が効くこと: 例外表から 0074 の行を一時的に外す
# → 「どの knowledge / specifications の sources からも参照されていない」で exit 1

# REQ 出典検査が効くこと: ev-kelly-bet-selection.md の sources から ADR を 1 本外す
# → 「出典 ... が frontmatter の sources に無い」で exit 1（従来は stale ごと消えて素通りだった）

# 回帰テスト
python3 scripts/test-check-doc-classes.py
```
