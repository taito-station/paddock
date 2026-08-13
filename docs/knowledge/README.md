# knowledge — 蒸留済み確定知の規約

dahatake/HypervelocityEngineering（HVE, MIT）の original-docs → qa → knowledge 蒸留モデルを
paddock に導入したもの。**蒸留は Claude Code が担う**（HVE 本体の LLM オーケストレータは持ち込まない）。

## 3 層モデル

```
docs/original-docs/  読み取り専用の一次資料（生素材 + ADR）
        │                              │
        │ [Claude が読取・欠落/不整合を検出]  │ ADR は qa を経由しない
        ▼                              │ （決定は既に確定しているため）
docs/qa/             質問票 + 回答       │
        │  [Claude が差分マージ]          │
        ▼                              ▼
docs/knowledge/ ＋ docs/specifications/   status 付き確定知（＝この層。読むのはここ）
```

- **横断検索**は mdq（Markdown Query, BM25・ローカル）で全 docs を索引する。生ファイルを読む前に
  `scripts/mdq search` を使う（[.claude/skills/markdown-query/SKILL.md](../../.claude/skills/markdown-query/SKILL.md)）。
- **ADR は一次資料層（`docs/original-docs/`）に属する不変の決定記録**（ADR 0073 で旧 `docs/adr/` から
  統合）。ADR も一次資料も「書き換えない（RO）」という性質が同じで、`ADR → knowledge` の写しを
  規約どおりの蒸留として扱えるため。**一度置いた ADR は改変しない**（決定を変えるときは新しい ADR で
  supersede する）。
- **確定知を読む入口は knowledge**。ADR の決定・理由・却下案・影響は knowledge に**全部写す**。
  重複を許す代わりに、同期切れ（`sources` が更新されたのに蒸留が追従していない状態）は
  **機械検査で検出する**——写した量に比例して stale 面積が増えるため、人手の規律には委ねない。
- > **現状（ADR 0073 の段階導入の到達点）**:
  >
  > - **stale の機械検査は配線済み・判定は error**（`scripts/check-doc-classes.py`・CI の `adr`
  >   ジョブと pre-push）。移設以前から累積していた未追従 6 件を
  >   [#580](https://github.com/taito-station/paddock/issues/580) で消化し、warning から昇格した。
  >   **これで「ADR の内容を knowledge へ全部写す」の担保が揃った**——写した先が追従漏れを
  >   起こせば CI が落ちる。
  > - **ADR の写しは一巡した**（#588）。**例外は ADR 0074 自身**——その決定（issue 本文を転記しない）は
  >   [docs/original-docs/README.md](../original-docs/README.md) の規約として反映済みだが、`sources` を持つ
  >   knowledge からは参照していないので stale 検査の対象外。ADR 77 本のうち、棄却 24 本は
  >   [product-goals.md](product-goals.md) が索引し、採用側はいずれかの knowledge / specifications が
  >   `sources` で参照して決定を写している。knowledge は 10 本。
  >   ただし**写しの粒度は一様ではない**ので、決定の細部（却下した代替案・数値の前提）が要るときは
  >   **ADR 原本（`docs/original-docs/0NNN-*.md`）も読む**。mdq は両方を索引しているので
  >   `scripts/mdq search` は横断で当たる。
  >
  > **順序は「機械検査の配線が先、写しは後」だった**。写した量に比例して stale 面積が増えるのが
  > ADR 0073 の出発点で、その前提条件（stale の error 化）は #580 で満たし、写しは #588 で一巡した。
  > 残るのは粒度を上げる作業なので、このブロックは「写しは一巡・粒度は不均一」の注記として残す。
- **`docs/original-docs/` の命名は 2 系統**（`check-adr-numbers.sh` の判定根拠。
  詳細は [docs/original-docs/README.md](../original-docs/README.md)）:
  - ADR = **0 埋め 4 桁**（`0001-`〜`0999-`）
  - issue 由来の一次資料 = **GitHub issue 番号（0 埋めしない）**（`382-`, `401-` …）

## knowledge はどこにあるか

- **`docs/specifications/`**: 既存のドメイン/機能知。**その場で knowledge に昇格**する（frontmatter を
  付与）。物理移動はしない——frontmatter を付けた時点で確定知層として機能し、`docs/knowledge/` へ
  移しても得られるものが無いため（ADR 0073 で実証したとおり移動コスト自体は小さいので、
  「リンクが多いから動かせない」わけではない）。
- **`docs/knowledge/`**: qa および ADR 由来の**新規・横断的な蒸留知**の置き場。既存 spec に属さない
  ものはここに置く。
- **語の定義を探すなら [glossary.md](glossary.md)（D07）から引く**。定義の正本がどの文書のどの節に
  あるかだけを持つ索引で、定義そのものは各仕様書・ADR・`CLAUDE.md` にある。

どちらも下記 frontmatter 規約に従い、mdq の索引対象（`mdq.toml`）に含める。

## frontmatter 規約

```yaml
---
status: Confirmed        # Confirmed（確定）/ Tentative（暫定）/ Conflict（矛盾・要解消）
kind: knowledge
doc_class: [D22, D24]    # 文書クラス。第 1 要素が主クラス。定義は docs/knowledge/doc-classes.md
tags: [D22, D24]         # doc_class の mdq 用ミラー（完全一致。checker が強制）
sources:                 # 由来。ADR / qa / original-docs のほか、確定知層（specifications /
                         # knowledge）や主題そのものであるファイル（ci.yml・openapi.json）も可。
                         # 判定は「その文書の本文が動いたら、この知の見直しが要るか」（ADR 0077）
  - docs/original-docs/0NNN-....md   # ADR は 0 埋め 4 桁
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
  割当索引と実ファイルの突合**・**REQ 表の `出典` ⊆ `sources`**・**orphan ADR** は **error**
  （stale は #580 で warning から昇格。リンクと割当索引は #604、後ろ 2 つは #596 / #597）。
  **サブディレクトリに置かれた `.md`** も error（ADR 0082 で warning から昇格。1 階層下げるだけで
  その文書が丸ごと無検査になり、`sources` も orphan 検査に数えられなくなるため）。
  **warning** は充足ギャップと、**判定不能を可視化する 2 経路**（`sources` の履歴を辿れない /
  shallow clone で `distilled_from_sha` を解決できない）。
  `--warn-only` は**ローカルで全件を眺めるための確認用**で、CI（`adr` ジョブ）も pre-push も
  フラグ無しで呼ぶので**これで CI を通すことはできない**（**検査そのものが成立しない 2 つ**——
  `doc-classes.md` のマーカー欠落と、ADR が 0 件——は `--warn-only` でも 1 で落ちる）。
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

要件・成功条件に**安定した参照子**を与える。ADR・issue・PR から `REQ-D01-004` の 1 語で名指しでき、
文書を書き換えても参照が壊れない。初出は [product-goals.md](product-goals.md)（D01 の成功条件）。

```markdown
<!-- REQ:begin D01 -->
| REQ-ID | 要件 | 検証手段 | 出典 | status |
|---|---|---|---|---|
| REQ-D01-001 | 張るレースは ROI ≥ 100% のものだけに限る | `paddock-predict --overview` の ROI | [ADR 0040](...) | Confirmed |
<!-- REQ:end D01 -->
```

- **形式は `REQ-D{NN}-{NNN}`**。`D{NN}` は [doc-classes.md](doc-classes.md) の文書クラス、`{NNN}` は
  3 桁ゼロ埋めの連番。クラスは**その要件を載せている文書のクラス＝番号空間の持ち主**を表す
  （関心事の分類ではない。買い方に関わる要件でも、D01 のプロダクト目標に載っていれば `REQ-D01-NNN`）。
- **一意性はクラス内グローバル**。同じ `D{NN}` の番号はリポジトリ全体で 1 つ。文書をまたいでも
  重複させない（同じクラスの REQ 表が複数文書に分かれてもよいが、番号空間は 1 つ）。
- **番号は再利用しない**。廃止した要件は行を消さず `status: Retired` にして残す。消して番号を空けると、
  過去の ADR / issue が指す `REQ-D01-003` が別の要件を指すようになる。
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

**ADR 側に REQ-ID は書かない。** ADR は RO なので後から ID を差し込めない——紐付けは knowledge 側の
`出典` 列が担う（ADR → REQ ではなく REQ → ADR の一方向）。

### 何が機械検査されるか

`scripts/check-doc-classes.py` が **error** で検査するのは次の範囲:

- マーカーの対応と書式、**マーカーの外にある REQ 表**、**行頭 `|` を欠いた REQ 行**、
  コードフェンスの閉じ忘れ（いずれも「表が丸ごと無検査になる」経路なので塞いである）
- 見出し行・区切り行・列数、REQ-ID の形式、ID のクラス部とブロックのクラスの一致
- ブロックのクラスが定義済みで、かつその文書の `doc_class` に含まれること
- 番号の重複、`status` の値域、`要件` / `出典` の非空、Confirmed の検証手段、リンク先の実在
  （リンクの実在検査は #604 で**本文にも**広げた。REQ 表の内外を問わず、相対リンクは実在必須）
- **`出典` 列が名指しした `docs/original-docs/` 配下のファイルが `sources` にも載っていること**
  （#597 / ADR 0082）。外部 URL・兄弟 knowledge へのリンク・リンク切れは対象外
- **ADR（0 埋め 4 桁）がどこかの knowledge / specifications の `sources` から参照されていること**
  （#596 / ADR 0082）。例外は `doc-classes.md` の `adr-orphan-exceptions` 表に理由付きで宣言する

一方、**次のものは機械検査できない**ので人手の規律に残る:

- **番号の再利用禁止**。検査が見るのは現時点のスナップショットだけなので、`Retired` 行ごと削除して
  同じ番号を別の要件に振り直しても検出されない。
- **`docs/knowledge/` と `docs/specifications/` の直下以外にある REQ 表**。検査対象はこの 2 ディレクトリの
  直下のみ（`README.md` を除く）で、`docs/original-docs/` や `CLAUDE.md` に REQ 表を置いても一意性の
  台帳には載らない。**REQ 表はこの 2 ディレクトリの中に置く**こと。
- **コードフェンスで囲んだ REQ ブロック**。フェンス内は「規約の見本」として全面的に無視する
  （この節の例がまさにそれ）。囲まれた表は GitHub でも表として描画されないので、実データを
  そこに置くことは無い前提。
- **`sources` の網羅性（残りの部分）**。stale 検査は「挙げた出典」に追従しているかしか見ないので、
  **`sources` から行を消せば stale も消える**（`sources` の変更自体はメタデータ扱いで下流にも
  伝播しない）。ADR 0082 で**両端は塞いだ**——REQ の `出典` が名指しした一次資料（#597）と、
  **ある ADR がどの `sources` からも参照されなくなる場合**（#596）は error になる。
  **まだ空いているのはその中間**で、REQ 表の外で本文が根拠にしている ADR や、参照元が複数
  あるうちの 1 本を `sources` から落とす操作は検出できない（導入時点で **ADR 81 本のうち
  48 本＝59% が 2 文書以上から参照**されており、そこは丸ごと中間にあたる）。
  **出典は減らさない**——減らすときは、その知がもうその ADR に依存していないことを本文で示す。
- **`出典` セルをリンクで書かなかった場合の突合**。検査 11 が見るのは Markdown リンクだけで、
  プレーンテキスト（`ADR 0001`）やインラインコードで書いた出典は突合されない。つまり
  **`sources` の行を消すより「出典のリンクを外す」ほうが安い回避策になった**。一律必須に
  しなかったのは、既存の出典に素のテキスト表記と外部 URL のみの行が実在するため（ADR 0082）。
- **ADR を `sources` に登録したことと、実際に写したことの区別**。#596 の検査が見るのは
  「どこかの `sources` に 1 行あるか」だけ。索引目的で `sources` に並べれば通るので、
  **決定・理由・却下案・影響を本文へ写したかは機械では分からない**。
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
  （＋ 各ディレクトリの `README.md` と、リポジトリルートの `CLAUDE.md`）だけ。**`docs/original-docs/`（ADR 原本）と `docs/qa/` は無検査**、
  サブディレクトリの `.md` も走査対象外（**置くこと自体が error**＝上記「機械検査」の項が正。
  severity をここに二重に書かない）。実在判定はファイルシステムを見るので、
  **git 管理外のパス**（生成物・gitignore 対象）へのリンクは手元で通り CI（fresh clone）で落ちる。
- **この README 自身の記述の鮮度**。README は frontmatter を持たない（`sources` も
  `distilled_from_sha` も無い）ので、`scripts/check-doc-classes.py` を書き換えても
  **ここが STALE にならない**。検査の仕様を変えたら、この節を手で直す。

## 昇格・更新の運用（Claude が回す蒸留）

1. 一次資料は `docs/original-docs/` に置く（RO・書き換えない）。
2. 調査で判明した Q&A は `docs/qa/` に質問票として起票し、回答を書き込む。
3. 回答済み qa と original-docs を突き合わせ、差分を knowledge に**差分マージ**（全書き換えしない・冪等）。
4. 矛盾は `status: Conflict` で明示し、解消してから `Confirmed` に上げる。
5. 決定を伴うものは ADR を `docs/original-docs/0NNN-*.md` に起票し（採番は
   `scripts/check-adr-numbers.sh next`）、knowledge の `sources` から参照する。**ADR の決定・理由・
   却下案・影響は knowledge へ全部写す**（読む入口を knowledge に一本化するため）。
   **新規 ADR の写しは起票と同じ PR で行う**——増える stale 面積が 1 本ぶんで、書いた本人がその場に
   いるうちに写すのが最も安い。既存 ADR の一括写しは #588 で一巡済み（差し止め条件だった stale の
   error 化は #580 で解消）。**#596 / ADR 0082 で機械検査になったのは「`sources` への登録」まで**
   ——どの `sources` からも参照されない ADR は error だが、**写しの中身（決定・理由・却下案・影響を
   実際に書いたか）は依然として人手の規律**（下の「機械検査できない」リスト参照）。写す先が無い
   ADR（規約そのものを定めた ADR / supersede 済み）は [`doc-classes.md`](doc-classes.md) の
   `adr-orphan-exceptions` 表に**理由付きで**登録する（`--warn-only` は逃げ道に数えない）。
   **`sources` に載せた時点で必ず STALE が出る**（自分の sha を同じコミットには書けない）ので、
   本文コミットの後に `scripts/bump-distilled-sha.py --all-stale` で追従コミットを積む。
   **knowledge / specifications を消す・統合する PR では、その文書の `sources` にある ADR を
   別文書へ移すか例外表に登録する**（割当索引と「現行」列の更新と同じ PR で）。消さなくても、
   1 文書の `sources` から ADR を落とすだけで orphan になりうる（導入時点で**単独参照の ADR が
   32 本**。ホーム自体は分散していて 1 文書あたり最大 5 本）。
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
   - **例外 2: `status: Conflict` の宣言だけを足すときは `updated` のみ bump し、
     `distilled_from_sha` は据え置く**。「乖離に気づいた」ことを記録するだけで、再蒸留は
     していないため。sha を進めると「その SHA の状態を反映している」ことになり、
     まさに乖離しているという事実と矛盾する。`Confirmed` に戻すとき（＝実際に差分マージした
     とき）に sha を現 HEAD へ進める。実例は [`app-bootstrap.md`](app-bootstrap.md)（解消は #578）。
