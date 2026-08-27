# knowledge — 蒸留済み確定知の規約

dahatake/HypervelocityEngineering（HVE, MIT）の docs-original → qa → knowledge 蒸留モデルを
paddock に導入したもの。**蒸留は Claude Code が担う**（HVE 本体の LLM オーケストレータは持ち込まない）。

## 2 層モデル

```
docs/docs-original/  読み取り専用の一次資料（実測ログ・調査所見・issue 由来の生素材）
        │
        │ [Claude が読取・欠落/不整合を検出]
        ▼
docs/qa/             質問票 + 回答
        │  [Claude が差分マージ]
        ▼
docs/knowledge/ ＋ docs/specifications/   status 付き確定知（＝この層。読むのはここ）
                                          末尾に **決定ログ**（append-only の決定記録）を持つ
```

蒸留フローとは別枠で `docs/docs-generated/` がある（HVE 由来。`cargo doc` / OpenAPI 等の自動生成文書の
置き場で、手書きの蒸留対象ではない）。

- **横断検索**は mdq（Markdown Query, BM25・ローカル）で全 docs を索引する。生ファイルを読む前に
  `scripts/mdq search` を使う（[.claude/skills/markdown-query/SKILL.md](../../.claude/skills/markdown-query/SKILL.md)）。
- **決定の記録は各文書の「決定ログ」節**（#652）。かつて `docs/docs-original/` に独立ファイルとして
  置いていた ADR は廃止し、決定・理由・却下案・影響は**その決定が効く knowledge / specifications の
  末尾**にある `## 決定ログ` 節へ集約した。**独立した ADR ファイルはもう作らない**——新しい決定は
  対応する確定知の決定ログに追記する。
- **確定知を読む入口は knowledge**。決定を辿るのも同じファイルの末尾で完結する（別ディレクトリの
  原本を突き合わせる必要が無い）。
- **一次資料層に残るのは転記できないもの**——実測ログ・調査時点のコード所見・外部サイトの挙動観察。
  ファイル名は **GitHub issue 番号（0 埋めしない）**（`382-`, `401-` …）。詳細は
  [docs/docs-original/README.md](../docs-original/README.md)。

## 決定ログの書き方

`docs/knowledge/` と `docs/specifications/` の各文書は、本文の末尾に `## 決定ログ` 節を持てる。

```markdown
---

## 決定ログ

<!-- この節は append-only です。既存エントリの変更・削除は CI が検出します。 -->

### ADR 0055: EV 層分離 (2026-07-02) — 承認済み

#### コンテキスト
#### 決定
#### 理由
#### 却下した代替案
#### 影響
```

- **見出しは `### ADR NNNN: 要約 (YYYY-MM-DD) — ステータス`**（旧 ADR 由来）または
  `### #NNN: 要約 (YYYY-MM-DD) — ステータス`（issue 番号採番の新規決定）。
  **旧 ADR の番号は見出しに残す**——`ADR 0055` のような既存の参照は `git grep 'ADR 0055'` で解決できる。
- **append-only**。既存エントリの本文を書き換えない・消さない（**CI が検出する**）。決定を変えるときは
  **新しいエントリを追記して、旧エントリを supersede した旨を新エントリに書く**。
- **どの文書に書くか**は「その決定が効く確定知」で決める。買い方の決定なら
  [ev-kelly-bet-selection.md](../specifications/ev-kelly-bet-selection.md)、文書運用の決定なら
  この README か [doc-classes.md](doc-classes.md)、というように**読む人が辿り着く場所**に置く。
  複数文書にまたがる決定は、主たる文書 1 本に書いて他からは相対リンクで指す（写しを増やさない）。
- **本文の規約（この節より上）と決定ログは別物**。本文は「今どうなっているか」を常に最新へ差分マージし、
  決定ログは「いつ・なぜそう決めたか」を積むだけ。決定が覆ったら**本文を直し、決定ログには
  新エントリを積む**（過去エントリは訂正しない）。

## knowledge はどこにあるか

- **`docs/specifications/`**: 既存のドメイン/機能知。**その場で knowledge に昇格**する（frontmatter を
  付与）。物理移動はしない——frontmatter を付けた時点で確定知層として機能し、`docs/knowledge/` へ
  移しても得られるものが無いため。
- **`docs/knowledge/`**: qa および一次資料由来の**新規・横断的な蒸留知**の置き場。既存 spec に属さない
  ものはここに置く。
- **語の定義を探すなら [glossary.md](glossary.md)（D07）から引く**。定義の正本がどの文書のどの節に
  あるかだけを持つ索引で、定義そのものは各仕様書・`CLAUDE.md` にある。

どちらも下記 frontmatter 規約に従い、mdq の索引対象（`mdq.toml`）に含める。

## frontmatter 規約

```yaml
---
status: Confirmed        # Confirmed（確定）/ Tentative（暫定）/ Conflict（矛盾・要解消）
kind: knowledge
doc_class: [D22, D24]    # 文書クラス。第 1 要素が主クラス。定義は docs/knowledge/doc-classes.md
tags: [D22, D24]         # doc_class の mdq 用ミラー（完全一致。checker が強制）
sources:                 # 由来。qa / docs-original のほか、確定知層（specifications /
                         # knowledge）や主題そのものであるファイル（ci.yml・openapi.json）も可。
                         # 判定は「その文書の本文が動いたら、この知の見直しが要るか」
  - docs/docs-original/NNN-....md    # issue 番号（0 埋めしない）
  - docs/qa/QA-....md
distilled_from_sha: "<short-sha>"  # この知が反映するリポジトリ状態の git SHA（トレーサビリティ）
updated: "YYYY-MM-DD"    # 内容を実質更新した日（YAML の date 型を避けるため必ずクォート。詳細な履歴は git log を正とする）
---
```

> **注意**: `updated` は必ずダブルクォートで囲む。クォートしないと YAML が `date` 型に解釈し、mdq の
> 索引化（frontmatter を JSON 化）が `Object of type date is not JSON serializable` で失敗する。

- **`doc_class` / `tags`**: 文書クラス（D01〜D24）の宣言。**定義とクラス一覧の正本は
  [doc-classes.md](doc-classes.md)**（書式・N/A 宣言・充足ギャップもそこ）。`tags` へのミラーは
  mdq が frontmatter を検索に使わず `tags` しか見ないため（`scripts/mdq search --tags D23`）。
  二重管理の drift は `scripts/check-doc-classes.py` が防ぐ。
- **機械検査**: 上記スクリプトが CI（`adr` ジョブ）と pre-push で走る。クラスの整合・`tags` の一致・
  `sources` の実在と表記の正規形・**stale**・**本文の相対リンクの実在**・**[doc-classes.md](doc-classes.md) の
  割当索引と実ファイルの突合**・**`sources` の履歴走査を完遂できなかったこと**・
  **REQ 表の `出典` ⊆ `sources`** は **error**
  （stale は #580 で warning から昇格。リンクと割当索引は #604、履歴走査の未完遂は ADR 0081、
  REQ 出典の突合は #597）。**決定ログの改変・削除**（append-only 違反）は別スクリプト
  `scripts/check-decision-log-immutability.py` が同じ経路で error にする（#652）。
  **サブディレクトリに置かれた `.md`** も error（1 階層下げるだけでその文書が丸ごと無検査に
  なるため）。**warning** は充足ギャップと、**判定不能を可視化する 2 経路**（`sources` の履歴が
  無い＝未コミット・履歴の尽き / shallow clone で `distilled_from_sha` を解決できない）。
  **「履歴が無い（warning）」と「走査を完遂できなかった（error）」は別物**——前者は環境の都合だが、
  後者は検査が回っていないので、warning に落とすと除外対象のコミットを積むほど検査が消える
  fail-open になる。
  `--warn-only` は**ローカルで全件を眺めるための確認用**で、CI（`adr` ジョブ）も pre-push も
  フラグ無しで呼ぶので**これで CI を通すことはできない**（**検査そのものが成立しない**
  `doc-classes.md` のマーカー欠落は `--warn-only` でも 1 で落ちる）。
  なお下記「機械検査できない」の `sources` の網羅性に注意——`sources` から行を消せば stale も消える。

- **status**: `Confirmed`=検証済みで運用の前提にしてよい / `Tentative`=検証中・暫定 /
  `Conflict`=source 間で矛盾があり要解消（放置しない）。
- **参照 SHA**: HVE `knowledge_versions.py`（参照 knowledge の git SHA を可視化）の軽量代替。
  原則は**この知を蒸留した時点のリポジトリ HEAD** を `git rev-parse --short HEAD` で記録する
  （pilot の `probability-estimation.md` もこの方式）。特定の由来ファイル版に紐付けたいときは
  `git log -1 --format=%h -- <path>` を使う。いずれも「いつ時点の知か」を辿れるようにするのが目的。
  **儀式化していないかは差分で数える**（#604 (e)）。subject の規約に頼ると取りこぼす——
  実際に main の sha 追従コミットは `docs(knowledge):` / `chore:` / `chore(spec):` と割れている。
  「`distilled_from_sha` 行しか触っていないコミット」＝見直しを伴わなかった追従として、
  過去に遡って数えられる:

  ```sh
  # docs 配下を触ったコミットのうち、sha 行しか変えていないものを列挙する
  # （--no-merges は必須。マージコミットは既定で差分が空になり、全部ヒットしてしまう）
  git log --no-merges --format=%H -- docs | while read -r c; do
    body=$(git show --format= --unified=0 "$c" -- docs \
      | grep -E '^[+-]' | grep -vE '^(\+\+\+|---)' | grep -v '^[+-]distilled_from_sha:')
    [ -z "$body" ] && echo "$c"
  done
  ```
  追従は `scripts/bump-distilled-sha.py <file>...`（`--all-stale` で STALE 全件）で 1 コマンドにできる
  ——**`updated` は触らない**ので、下流の本文が実質変わったかは自分で判断して手で進める。
  なお**同一コミットに自分の sha は書けない**ので、上流を触った PR は「本文コミット →
  sha 追従コミット」の 2 コミットになる。**その後に `rebase -i` や `--amend` で履歴を書き換えたら
  bump し直す**（記録した sha が push されず、CI の fresh clone で「解決できない」になる）。
- **変更履歴**: **git log を正とする**（変更の追跡は履歴で辿る）。内容を実質更新したら `updated` と
  `distilled_from_sha` を更新すれば足りる。本文末尾の `## 変更履歴` セクションは**任意**——
  節目や意図を人間可読に残したいときだけ置く（一括後付けはしない）。既に `## 変更履歴` を持つ 2 本
  （[`docs/specifications/probability-estimation.md`](../specifications/probability-estimation.md) /
  [`docs/knowledge/analyze-search-and-state.md`](analyze-search-and-state.md)）はそのまま維持してよい。

## REQ-ID（要件 ID）の規約

要件・成功条件に**安定した参照子**を与える。決定ログ・issue・PR から `REQ-D01-004` の 1 語で名指しでき、
文書を書き換えても参照が壊れない。初出は [product-goals.md](product-goals.md)（D01 の成功条件）。

```markdown
<!-- REQ:begin D01 -->
| REQ-ID | 要件 | 検証手段 | 出典 | status |
|---|---|---|---|---|
| REQ-D01-001 | 張るレースは ROI ≥ 100% のものだけに限る | `paddock-predict --overview` の ROI | [product-goals.md 決定ログ ADR 0040](product-goals.md) | Confirmed |
<!-- REQ:end D01 -->
```

- **形式は `REQ-D{NN}-{NNN}`**。`D{NN}` は [doc-classes.md](doc-classes.md) の文書クラス、`{NNN}` は
  3 桁ゼロ埋めの連番。クラスは**その要件を載せている文書のクラス＝番号空間の持ち主**を表す
  （関心事の分類ではない。買い方に関わる要件でも、D01 のプロダクト目標に載っていれば `REQ-D01-NNN`）。
- **一意性はクラス内グローバル**。同じ `D{NN}` の番号はリポジトリ全体で 1 つ。文書をまたいでも
  重複させない（同じクラスの REQ 表が複数文書に分かれてもよいが、番号空間は 1 つ）。
- **番号は再利用しない**。廃止した要件は行を消さず `status: Retired` にして残す。消して番号を空けると、
  過去の決定ログ / issue が指す `REQ-D01-003` が別の要件を指すようになる。
- **マーカーで囲む**。`<!-- REQ:begin D{NN} -->` … `<!-- REQ:end D{NN} -->`。表の位置を本文構造に
  依存させないため（見出しを変えても検査が壊れない）。マーカーのクラスは**その文書の `doc_class` に
  含まれていること**——他クラスの要件を勝手に抱え込ませない。
- **列は 5 列固定**（`REQ-ID | 要件 | 検証手段 | 出典 | status`）。**見出し行と区切り行も必須**で、
  順序も変えない。列は位置で意味付けして読むので、順序が入れ替わると下記の Confirmed 検査が別の列に
  当たる。書式が崩れた行は黙って落とさず error にする（落とすと一意性検査から消えて重複が通る）。
  セル内にパイプを書くときは GFM どおり `\|` とエスケープする。
- **`status` は `Confirmed` / `Tentative` / `Conflict` / `Retired`**。前 3 つは frontmatter の `status` と
  同じ意味で、`Retired` は「かつて要件だったが取り下げた」。
- **`要件` と `出典` は空にできない**。`出典` と `検証手段` に書く Markdown リンクは**実在すること**
  （絶対パス・リポジトリ外は不可。リポジトリ内なら `docs/` の外＝`scripts/` 等でもよい）——由来を
  辿れない要件は根拠を確認できない。インラインコード（`` `…` ``）の中はコマンドとして扱い、検査しない。
- **検証手段が空なら `Confirmed` にできない**。「達成した」と言えるのは測り方が決まっているときだけで、
  検証手段の無い Confirmed は願望と区別が付かない。空欄のほか
  `-` / `–` / `—` / `TBD` / `UNKNOWN` / `n/a` / `未定` / `なし` / `未整備` も空扱い（大小文字は問わない）。

### REQ 表のある文書（索引）

番号空間はクラス内グローバルなので、**新しい REQ を採番する前にここを見る**。

| クラス | 文書 | 範囲 |
|---|---|---|
| D01 | [product-goals.md](product-goals.md) | 目標の成功条件（ROI ゲート・層分離・軸ロック・精度水準・提示形式） |
| D22 | [probability-estimation.md](../specifications/probability-estimation.md) | 本番構成の定数（α / m / 冪較正 / trend_n / 各 factor の重み） |
| D23 | [ev-kelly-bet-selection.md](../specifications/ev-kelly-bet-selection.md) | 買い方の具体（券種構成・相手幅・配分・Kelly の用途・混戦判定・Python の禁止用途）。**ROI ゲート / 軸ロック / 提示形式は D01 が正本** |

**決定ログのエントリに REQ-ID は書かない。** 決定ログは append-only なので後から ID を差し込めない
——紐付けは REQ 表の `出典` 列が担う（決定 → REQ ではなく REQ → 決定の一方向）。

### 何が機械検査されるか

`scripts/check-doc-classes.py` が **error** で検査するのは次の範囲:

- マーカーの対応と書式、**マーカーの外にある REQ 表**、**行頭 `|` を欠いた REQ 行**、
  コードフェンスの閉じ忘れ（いずれも「表が丸ごと無検査になる」経路なので塞いである）
- 見出し行・区切り行・列数、REQ-ID の形式、ID のクラス部とブロックのクラスの一致
- ブロックのクラスが定義済みで、かつその文書の `doc_class` に含まれること
- 番号の重複、`status` の値域、`要件` / `出典` の非空、Confirmed の検証手段、リンク先の実在
  （リンクの実在検査は #604 で**本文にも**広げた。REQ 表の内外を問わず、相対リンクは実在必須）
- **`出典` 列が名指しした `docs/docs-original/` 配下のファイルが `sources` にも載っていること**
  （#597 / ADR 0083）。外部 URL・兄弟 knowledge へのリンク・リンク切れは対象外

加えて `scripts/check-decision-log-immutability.py` が **決定ログの append-only 性**を error で
検査する（#652。既存エントリの改変・削除を検出する）。

一方、**次のものは機械検査できない**ので人手の規律に残る:

- **番号の再利用禁止**。検査が見るのは現時点のスナップショットだけなので、`Retired` 行ごと削除して
  同じ番号を別の要件に振り直しても検出されない。
- **`docs/knowledge/` と `docs/specifications/` の直下以外にある REQ 表**。検査対象はこの 2 ディレクトリの
  直下のみ（`README.md` を除く）で、`docs/docs-original/` や `CLAUDE.md` に REQ 表を置いても一意性の
  台帳には載らない。**REQ 表はこの 2 ディレクトリの中に置く**こと。
- **コードフェンスで囲んだ REQ ブロック**。フェンス内は「規約の見本」として全面的に無視する
  （この節の例がまさにそれ）。囲まれた表は GitHub でも表として描画されないので、実データを
  そこに置くことは無い前提。
- **`sources` の網羅性**。stale 検査は「挙げた出典」に追従しているかしか見ないので、
  **`sources` から行を消せば stale も消える**（`sources` の変更自体はメタデータ扱いで下流にも
  伝播しない）。塞いであるのは REQ の `出典` が名指しした一次資料だけ（#597 / ADR 0083）で、
  **REQ 表の外で本文が根拠にしている一次資料を `sources` から落とす操作は検出できない**。
  **出典は減らさない**——減らすときは、その知がもうその資料に依存していないことを本文で示す。
- **`出典` セルをリンクで書かなかった場合の突合**。検査 11 が見るのは Markdown リンクだけで、
  プレーンテキスト（`ADR 0001`）やインラインコードで書いた出典は突合されない。つまり
  **`sources` の行を消すより「出典のリンクを外す」ほうが安い回避策になった**。一律必須に
  しなかったのは、既存の出典に素のテキスト表記と外部 URL のみの行が実在するため（ADR 0083）。
- **決定ログの中身**。append-only 検査が見るのは「既存エントリが改変・削除されていないか」だけで、
  **新しい決定を決定ログに書いたかどうか・書いた内容が十分か（コンテキスト・理由・却下案・影響）は
  機械では分からない**。ここは人手の規律に残る。
- **リンクの指し先の「中身」**。実在するのはファイル／ディレクトリまでで、**`#` 以降のアンカーと
  散文で書いた節名（「〜」節・「ステップ 4: 指標集計」等）は未検査**。`foo.md#存在しない節` は通る。
  節名は表記ゆれと部分一致で誤検知が出やすいため、意図して人手に残している（#604）。
- **索引の網羅性（`doc-classes.md` の割当索引を除く）**。上記「REQ 表のある文書（索引）」と
  `doc-classes.md` の「充足ギャップ」表は手書きで、実態との突合は無い。
- **インライン形式以外のリンク**。`[label]: path` の参照形式定義と HTML の `<a href>` は
  検査対象外（現状 docs に 0 件）。**4 スペースインデントのコードブロック**も除外していないので、
  その中の見本リンクは実データとして検査される（フェンスで囲めば除外される）。
  逆に**行を跨ぐインラインコード**（`` ` `` で囲んだ中に改行を含む書き方）は「コード」と
  見なさないので、その中のリンク様文字列は実データとして検査される。本文はフェンス外を
  連結して走査するので、**閉じ忘れた `[` が離れた行の `]` と対になる**と、error の行番号が
  実際の記述とずれることがある（実在判定自体は誤らない）。
- **リンク検査の適用範囲**。見るのは `docs/knowledge/` と `docs/specifications/` の**直下**
  （＋ 各ディレクトリの `README.md` と、リポジトリルートの `CLAUDE.md`）だけ。**`docs/docs-original/` と `docs/qa/` は無検査**、
  サブディレクトリの `.md` も走査対象外（**置くこと自体が error**＝上記「機械検査」の項が正。
  severity をここに二重に書かない）。実在判定はファイルシステムを見るので、
  **git 管理外のパス**（生成物・gitignore 対象）へのリンクは手元で通り CI（fresh clone）で落ちる。
- **この README 自身の記述の鮮度**。README は frontmatter を持たない（`sources` も
  `distilled_from_sha` も無い）ので、`scripts/check-doc-classes.py` を書き換えても
  **ここが STALE にならない**。検査の仕様を変えたら、この節を手で直す。

## 昇格・更新の運用（Claude が回す蒸留）

1. 一次資料は `docs/docs-original/` に置く（RO・書き換えない）。
2. 調査で判明した Q&A は `docs/qa/` に質問票として起票し、回答を書き込む。
3. 回答済み qa と docs-original を突き合わせ、差分を knowledge に**差分マージ**（全書き換えしない・冪等）。
4. 矛盾は `status: Conflict` で明示し、解消してから `Confirmed` に上げる。
5. **決定を伴うものは、その決定が効く knowledge / specifications の `## 決定ログ` に 1 エントリ追記する**
   （書式は上記「決定ログの書き方」）。**独立した ADR ファイルは作らない**（#652）。
   同じ PR で**本文側も直す**——決定ログは「いつ・なぜ決めたか」、本文は「今どうなっているか」で、
   読む人が最初に見るのは本文のほう。決定ログだけ積んで本文を古いまま残さない。
   **エントリは書いた本人がその場で書き切る**（コンテキスト・決定・理由・却下した代替案・影響）。
   ここは機械検査が届かない範囲で、後から補完する機会は実質来ない。
6. **sources 追従**: knowledge の `sources` に列挙されたファイルを**内容ごと**変更する PR は、参照元
   knowledge の `distilled_from_sha` を現 HEAD に更新する（**機械検査の対象はこちらだけ**）。本文が変わる場合は差分マージを
   行って `updated` も進め、**本文が変わらない場合は `distilled_from_sha` の bump のみ**（`updated` は
   「内容を実質更新した日」なので据え置く。例外 1c と同じ理屈で、下流の本文に効かない上流変更まで
   日付を進めると「いつ確定した知か」の信号が濁る）。**この追従は機械検査の対象**（実例:
   [`app-bootstrap.md`](app-bootstrap.md) が `status: Confirmed` のまま、qa 側で「#453 で覆る」と
   追記済みの `NoopParser` を推奨し続けた事故がある。人手の規律だけでは守れない）。
   - **例外 1: パス移動のみ（内容不変）は bump しない**。`sources` の行が指す先が同じ内容のまま
     別パスへ移っただけなら、その knowledge が反映するリポジトリ状態は変わっていない。ここで
     `distilled_from_sha` を進めると「その SHA 時点で蒸留し直した」という偽の主張になり、
     `updated`（＝内容を実質更新した日）の定義とも矛盾する。ADR 0073 の ADR 移動がこのケースで、
     20 本の `sources` パスを書き換えたが sha / 日付は据え置いた。
     機械検査は **`R100`（内容差分ゼロのリネーム）のコミットを飛ばして実質の変更点まで遡る**
     ことでこの例外を吸収する。`git log --follow` 単体では吸収できない——`--follow` はリネームより
     前へ履歴を遡らせるだけで、「最終コミット」がリネームコミットになる事実は変わらないため、
     そのまま比較すると 20 本すべてが stale 判定になる。
   - **例外 1b: frontmatter のメタデータだけの変更は「内容変更」と見なさない**。
     `doc_class` / `tags` / `sources` / `distilled_from_sha` / `updated` のいずれかが変わっても、
     本文が同一ならその文書の内容は変わっていない。移設に伴う `sources` のパス追従や文書クラスの
     付与がこれで、除外しないと**それらを `sources` に持つ knowledge が軒並み stale になる**
     （実測: ADR 移設で 7 件、文書クラス付与で 6 件）。機械検査は本文と frontmatter を分けて
     比較し、差分がメタデータキーだけなら遡る。`status` / `kind` は除外しない——
     `Confirmed → Conflict` は下流へ伝えるべき信号なので、内容変更として扱う。
   - **例外 1c: それでも残る移設由来の stale は `distilled_from_sha` を再ベースラインする**
     （`updated` は触らない）。本文中の相対リンクまで書き換わった場合は 1b では吸収できない。
     内容が実質変わっていないなら、`distilled_from_sha` を移設後の HEAD へ進めるのが正確
     （その SHA の状態を反映しているのは事実）。`updated` は内容更新日なので据え置く。
     **再ベースラインしてよいのは、stale の原因が移設だけの文書に限る**——ほかに実質更新が
     混じっているものを一緒に進めると、本物の信号を消してしまう。
   - **例外 1d: `uses:` のピン留め SHA 更新だけの差分は「内容変更」と見なさない**（ADR 0081）。
     **例外 1 / 1b と同じ「機械が吸収する」側の例外**（人が bump する 1c とは性質が違う）。
     `.github/workflows/ci.yml` は [`ci-pipeline.md`](ci-pipeline.md) の `sources` だが、
     そこが語るのは**ジョブ構成と分割の設計意図であって各 action の版ではない**ので、ピンの hex が
     上がっても下流に読み直す理由が無い。除外しないと **dependabot の Actions 更新 PR が構造的に
     永久に赤**になる（dependabot は自分のエコシステム外の frontmatter を編集できない。実害:
     #590 / #591 が 2 日以上塞がれ、#607 で人が手で統合した）。例外 1b では吸収できない——
     `is_metadata_only_change` は frontmatter に依存し、先頭行が `---` でない `.yml` では
     構造的に効かない。機械検査が遡るのは次の**すべて**を満たすときだけ:
     (1) 対象パスが `.github/workflows/*.yml` または `*.yaml`（判定は行単位・字面ベースで YAML 構造を
     見ないので、絞らないと Markdown のフェンス内に書いた `uses:` の見本を書き換えただけで検査が消える）/
     (2) 変更前後で行数が同じ / (3) 差分行が **1 行以上**あり、すべてが `uses:` のピン行 /
     (4) 各行でインデント・`uses:`・owner/repo（`/` を含まない 2 要素）が同一 /
     (5) 少なくとも 1 行で 40 hex が実際に変わっている。
     **変わってよいのは 40 hex と末尾のバージョン注記（`# v4` → `# v7.0.0`。dependabot が
     書き換えることがある）だけ**で、action の差し替え・再利用可能ワークフロー参照の更新・
     タグへの緩和・行の増減・注記の散文化・改行コードのみの変更はいずれも内容変更のまま。
   - **例外 2: `status: Conflict` の宣言だけを足すときは `updated` のみ bump し、
     `distilled_from_sha` は据え置く**。「乖離に気づいた」ことを記録するだけで、再蒸留は
     していないため。sha を進めると「その SHA の状態を反映している」ことになり、
     まさに乖離しているという事実と矛盾する。`Confirmed` に戻すとき（＝実際に差分マージした
     とき）に sha を現 HEAD へ進める。実例は [`app-bootstrap.md`](app-bootstrap.md)（解消は #578）。

---

## 決定ログ

<!-- この節は append-only です。既存エントリの変更・削除は CI が検出します。 -->

### ADR 0074: 一次資料層に GitHub Issue 本文を転記しない (2026-08-10) — 承認済み

#### ステータス

承認済み。ADR 0073 の後続（[#579](https://github.com/taito-station/paddock/issues/579) の段階的タスク）。

#### コンテキスト

`docs/docs-original/` は「読み取り専用の一次資料（RO）」の層で、蒸留の出発点を固定するために置いている。
ところが issue 由来の一次資料 4 本は、いずれも **GitHub Issue 本文を逐語転記した章**を先頭に持っていた。

ADR 0073 のために全文照合したところ、実測は次のとおりだった。

- 4 本すべてが **Issue 本文の 25〜38% を逐語コピー**していた。
- しかも**原本と一致していない**:
  - `384-analyze.md` は「別 issue」という記述を「#379・実装済」に**書き換えていた**
  - `389-race-name.md` は「現状」章を**削除**していた
  - `401-analyze-partial-match.md` は「要件」章 4 項目を**削除**していた
- つまり転記は**原本として機能しておらず**、RO 原則で守っているのは「原本」ではなく「劣化コピー」だった。

問題の構造は、`sources` 追従（ADR 0073 決定 2）と同じ「二重管理」だが、**こちらは機械検査が原理的に効かない**。
GitHub 上で issue 本文が編集されても git には何も現れないため、乖離を検出する手段が無い。

#### 決定

**`docs/docs-original/` に GitHub Issue 本文を転記しない。** 原本は GitHub Issue とし、リポジトリ側は
**リンクと取得コマンドの数行だけ**を置く。

```markdown
## 発端の Issue

原本は [#382](https://github.com/taito-station/paddock/issues/382)（転記しない・ADR 0074）。
本文は `gh issue view 382` で取得する。
```

1. **転記済みの 4 本（`382` / `384` / `389` / `401`）から該当章を削除し、上記のリンクに置き換える。**
   これは **RO 原則に対する明示的な例外**として本 ADR で承認する（README の例外リストに追加する）。
2. **調査所見・実測・生ログは一次資料として残す。** 「コード調査所見（対象 SHA 明記）」「現状の確認（実測）」
   「mdq 探索ログ」「`pmset -g log` 抜粋」などは **GitHub には無く、ここにしか存在しない**素材で、本 ADR の
   対象外。むしろこれが一次資料層の本来の中身。
3. **issue 本文の内容が蒸留に必要なら、qa / knowledge 側に「その時点の要求」として引用する。**
   引用元と日付を明記すれば、原本が後で変わっても「いつ時点の要求で判断したか」が残る。

#### 理由

- **RO で守る価値があるのは原本だけ**。原本と食い違うコピーを凍結しても、辿れるのは誤りだけになる。
  実測で 3/4 が改変・章削除だったので、これは理論上の懸念ではなく既に起きている事故。
- **同期を機械検査できない**。ADR 0073 は「重複を許す代わりに機械検査で担保する」を選んだが、その担保は
  git の履歴があって初めて成り立つ。GitHub 側の編集は git に現れないので、同じ手が使えない。
  **担保できない重複は作らない**、が一貫した判断になる。
- **リンクは陳腐化しない**。issue 番号は不変で、`gh issue view` はいつでも最新の原本を返す。
  「蒸留の出発点を固定する」という目的は、転記ではなく**参照の固定**で達成できる。
- **一次資料層の本当の価値は転記できないものにある**。実測ログ・調査時点のコード所見・外部サイトの挙動観察は
  他のどこにも無い。転記章を削ることで、その価値が埋もれなくなる。

#### 却下した代替案

- **転記を続け、定期的に原本と同期する**。読む側はリポジトリ内で完結できるが、同期漏れを検出する機械検査が
  作れない（GitHub 側の変更が git に出ない）。人手の規律に委ねる形になり、ADR 0073 が「守れない」と
  実証した経路そのもの。
- **転記を残したまま「原本ではない」と注記する**。変更量ゼロで済むが、読み手が最初に目にするのは本文なので
  注記は事故を防がない。`app-bootstrap.md` の `NoopParser` 事故（`status: Conflict` の警告があっても本文が
  読まれた）と同型。
- **issue 本文を取得日時付きスナップショットとして保持する**。「いつ時点の要求か」は残るが、乖離そのものは
  残り、さらに「いつ更新すべきか」という責務が新たに生まれる。必要なときに qa / knowledge へ引用する
  （決定 3）方が、責務を増やさずに同じ情報を残せる。
- **issue 由来の一次資料をディレクトリごと廃止する**。転記が無くなるなら調査所見だけになるが、それらは
  ADR にも knowledge にも属さない生素材で、置き場所が必要。

#### 影響

- **変更（RO の例外）**: `382-live-server-now.md` / `384-analyze.md` / `389-race-name.md` /
  `401-analyze-partial-match.md` の issue 本文章を削除しリンクに置換。**調査所見・実測・mdq 探索ログは無変更**。
- **不変**: `568-monitor-sleep-gap.md`（`pmset -g log` 抜粋などの生ログのみで、転記章を持たない）。
  ADR 73 本も対象外（ADR は issue のコピーではない）。
- **変更**: `docs/docs-original/README.md` の「何を置かないか」に「GitHub Issue 本文の転記」を追加し、
  RO の例外リストに本 ADR を加える。
- **追従**: 4 本を `sources` に持つ knowledge / specifications は stale になる。本文の削除は蒸留済み内容に
  影響しない（転記章は knowledge へ写していない）ため、`distilled_from_sha` の再ベースラインで足りる
  （[docs/knowledge/README.md](../knowledge/README.md) 例外 1c）。
- **運用**: 今後 issue 由来の一次資料を新設するときは、リンク 1 行 + 調査所見の形にする。
- 関連: ADR 0073（一次資料層への ADR 統合・`sources` 追従の機械検査）/ #579。

#### 再現方法

```sh
# 転記章が残っていないこと（issue 本文の見出しが 0 件）
git grep -nE '^#+ (Issue #[0-9]+ 概要|[0-9]+\. issue #[0-9]+ 本文)' -- docs/docs-original   # → 0 行

# 原本はいつでも取れる
gh issue view 382 --repo taito-station/paddock
```

### #652: ADR 廃止と決定ログ統合 (2026-08-23) — 採用

#### コンテキスト

90 本の ADR ファイルと knowledge/specifications の二重管理のコストが実利を上回っていた。ADR の内容は knowledge に「全部写す」ことで二重正本化し、orphan 検査・STALE 追従・bump-distilled-sha 等の機械検査で整合を維持していたが、6 行の CI 変更でも追従コミットが必要になるなど日常的なオーバーヘッドが発生していた。

#### 決定

ADR を独立した文書種別として廃止し、決定・理由・却下案・影響を各 knowledge/specifications の「決定ログ」節に集約する。決定ログは append-only（既存エントリの変更・削除は CI が検出する）。ADR 番号は決定ログ見出しに残し、既存の番号参照は grep で解決可能。

#### 理由

- 二重正本の維持コスト（orphan 検査、STALE 追従、bump コミット）が日常的に発生していた
- ADR は「不変の決定記録」だが、knowledge に全部写す以上、知識層にも同じ情報がある
- 決定ログを append-only にする機械検査で不変性を担保できる

#### 影響

- docs/docs-original/ には非 ADR の一次資料（実測ログ・issue 由来）11 本のみ残る
- 旧 ADR 番号（ADR 0001〜0090）は各ファイルの決定ログ見出しから検索可能
- check-adr-numbers.sh / check-doc-classes.py の orphan 検査は撤去
- 新規の決定は知識文書の決定ログ節に直接 append する（ADR ファイルは作らない）
