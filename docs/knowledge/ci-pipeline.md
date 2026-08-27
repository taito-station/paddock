---
status: Confirmed
kind: knowledge
doc_class: [D21, D19, D17]
tags: [D21, D19, D17]
sources:
  - docs/docs-original/616-docs-serving-checks.md
  - docs/docs-original/636-fullwidth-after-var.md
  - docs/qa/QA-evil-merge-615.md
  - docs/qa/QA-fullwidth-after-var-636.md
  - .github/workflows/ci.yml
distilled_from_sha: "daf3beb"
updated: "2026-08-22"
---

# CI パイプラインの構成と設計意図（D21）

`.github/workflows/ci.yml` の**ジョブ分割がなぜこの形なのか**を書く。ジョブ一覧はワークフローを見れば
分かるが、「なぜ分けたか」「なぜこの版に固定したか」はコードに書けないのでここが正になる。

D21（CI/CD・ビルド・リリース・供給網管理）の充足ギャップを埋める文書
（[doc-classes.md](doc-classes.md) 参照）。

## ジョブ構成（8 ジョブ + マトリクス 3）

| ジョブ | 実行環境 | 内容 |
|---|---|---|
| `ci` | ubuntu-latest ＋ postgres サービス | toolchain 一致 assert / **Swagger UI vendored 検査**（回帰テスト → 本番検査）/ fmt / clippy / `cargo test`（**直列**・OCR・PDF crate を除く） |
| `web` | ubuntu-latest | typecheck / eslint / vitest / **生成 API 型のドリフト検証** / vite build |
| `adr` | ubuntu-latest | ADR 番号重複と文書クラス・sources の検査（**回帰テスト → 本番検査**の順） |
| `predict-check` | ubuntu-latest | stdlib のみの Python テスト（自走式 + ハーネス忠実性） |
| `shellcheck` | ubuntu-latest | `shellcheck --severity=warning` ＋ **変数直後の非 ASCII 検査**（回帰テスト → 本番検査）＋ `keep_awake.sh` の回帰テスト（#585/#643）＋ `prefetch_odds.sh` の lock 回帰テスト（#651・PATH を絞って本番の mkdir 経路も ubuntu で踏ませる） |
| `db-guards` | ubuntu-latest（**postgres サービス無し**・`postgresql-client` のみ） | golden DB ガードの回帰テスト（#406/#465）。到達不能ポートを使い実 DB を一切触らない設計なので DB サービスが要らない |
| `ocr-pdf` | ubuntu-latest ＋ **`debian:trixie-slim` コンテナ** | mupdf 依存の `pdf-ocr` / `pdf-parser` 統合テスト |
| `docker-build` | ubuntu-latest（matrix 3） | api / importer / web の Dockerfile の builder ステージをビルド |

## 設計意図

### なぜ `ocr-pdf` だけコンテナで分離するのか（ADR 0026）

**mupdf の版を固定できる場所が必要**だから。`MutoolParser` は mupdf 1.25 以上でないと成績 PDF を
解析できず、**下限を割ると 0 レースになる**（例外にならず静かに空になる）。

- `ubuntu-latest` の apt に入る mupdf は 1.23 で**不足**。ソースビルドは CI を重く・脆くする。
  `debian:trixie-slim` は apt 一発で 1.25.1 が入り、**イメージタグで版を固定できる**。
- **本体ジョブ（`ci`）を丸ごとコンテナ化しない**。`ci` は Postgres サービス + `localhost` 前提で、
  container 化するとサービスネットワーク（`localhost` → サービス名）と DB 接続を作り直すことになる。
  PDF テストは DB を触らないので、別ジョブに切る方が安全かつ並列で速い。
- **`mutool` のバージョン下限を assert するゲートステップ**をテストの前に置く。版がドリフトしたとき、
  サイレントに 0 レース化させず明示的に落とす——この検査が無いと「テストは緑だが解析は空」になる。
- `--test-threads=1` で走らせるのは、複数テストバイナリが並行して JRA 取得に行くのを避け、出力を
  決定的にするため。

### コンテナイメージは tag 参照（digest ピンしない）

外部 action は SHA ピンするが、**コンテナイメージは tag 参照**にする（`ci` の `postgres:17-alpine` と
同じ扱い）。OS イメージはセキュリティ更新を取り込みたく、digest 固定は陳腐化と手動更新の負担が大きい。
版ドリフトの実害（mupdf が下限割れ）は上記の assert gate が検知するので、ピンで防ぐ必要が無い。

### stdlib スクリプトは CI の python3（ubuntu-latest 同梱・版はピンしない）で動くこと

`scripts/*.py`（checker・bump・各回帰テスト）は CI では **ubuntu-latest 同梱の python3** で走る
（`setup-python` を使っていないので版はイメージ任せ）。手元の macOS の方が新しいと、新しい版で
入った API（例: `Path.read_text(newline=...)` は 3.13 以降）を使ってもローカルは緑のまま CI だけが
落ちる。**新しい stdlib API を使うときは追加バージョンを確認する**（#604 で実際に踏んだ）。

### `adr` ジョブは回帰テストを本番検査より先に走らせる

検査が落ちたとき、**ADR が本当に重複しているのか判定器が壊れているのか**を切り分けられるようにする
（ADR 0073）。fail-closed を謳う検査ほど、壊れても本番データが正常なら気づけない。
**同じ順序を `shellcheck` ジョブと pre-push の各検査にも適用している。**

### 変数直後の非 ASCII を静的に禁じる（#636）

`$var` の直後に全角括弧などを置くと、**UTF-8 ロケールの bash がそのバイトまで変数名に取り込み**、
`set -u` で `unbound variable` になって落ちる（識別子の終端判定が `isalnum()` ＝ロケール依存のため）。
**`shellcheck 0.11.0` は検出しない**ので `scripts/check-shell-var-nonascii.sh` を置く。対象ファイル集合は
同ジョブの `shellcheck` と同一にして二重管理を避ける。

**実行時テストではなく静的検査にしたのが要点。** 挙動は `LC_ALL=C` なら正常・UTF-8 なら失敗という
ロケール依存で、Linux/glibc での再現有無も未確認。実挙動をアサートすると環境ごとに結果が割れる。
字面で禁じれば、**仮に CI（ubuntu）で再現しなくても macOS 側の事故を止められる**。

**なぜ CI 任せにせず pre-push にも置くか**: launchd の plist は `PATH` しか設定しない＝C ロケールなので
常駐ジョブは壊れず、**壊れるのは人が UTF-8 の端末から叩いたとき**。開催日の運用スクリプトがそれに当たる
（2026-08-16 に `deployments/launchd/uninstall.sh` が実際に途中で止まった）。

行頭コメントは除外する——**この罠を説明するコメントで悪い例を書けるようにする**ため
（`scripts/test-check-adr-numbers.sh`）。行末コメントは字句解析が要るので検出側に倒す。

**既知の非カバー範囲**（「検査済み」と誤解しないための記録）: 行頭コメント除外は「その行が本当に
コメントか」を見ていないので、**展開される複数行文脈**（クォート無しヒアドキュメント本文・複数行
ダブルクォート文字列の継続行・`$(...)` の中）の `#` 始まり行は素通りする。`.github/workflows/*.yml`
の `run:` と Dockerfile の `RUN`、Markdown 内の実行用フェンスも対象外。
**「現時点で該当 0 件」は点検時点の観測であって保証ではない。** 詳細は検査スクリプトのヘッダ。

### dependabot の Actions ピン更新は stale 検査の例外にする（ADR 0081）

**このワークフローは本文書の `sources` に入っているので、`ci.yml` を触る PR は `adr` ジョブの
stale 検査を通らない**——`distilled_from_sha` の追従コミットが要る。ところが dependabot は自分の
エコシステム外のファイル（下流 knowledge の frontmatter）を編集しないため、**Actions の SHA ピン
更新 PR は構造的に永久に赤**だった。実害として #590（dtolnay/rust-toolchain）と
#591（Swatinem/rust-cache）が 2026-08-10 から 2 日以上マージできず、2 本とも `ci.yml` を触るので
片方をマージするともう片方が必ずコンフリクトする状態になり、最終的に人が #607 で 1 本に統合して
「`ci.yml` コミット → sha 追従コミット」の 2 コミットを手で積んだ。SHA ピンはサプライチェーン対策
なのに、その更新経路が機械的に塞がっているのは本末転倒で、セキュリティ更新の停滞に直結する。

**決定**: `uses:` 行のピン留め SHA 更新だけの差分を「内容変更」から外す
（[README.md](README.md) の**例外 1d**。例外 1 / 1b と同じ「機械が吸収する」側の例外）。判定は
`scripts/check-doc-classes.py` の `is_pin_only_change` が行い、`last_content_change` が
例外 1 / 1b と同じ位置で呼んで該当コミットを飛ばす。真になるのは次のすべてを満たすときだけ:
(1) 対象パスが `.github/workflows/*.yml`（`*.yaml` も可）、(2) 変更前後で行数が同じ、
(3) 差分行が 1 行以上あり**すべて**が `uses: <owner>/<repo>@<40hex>` 形式
（owner/repo は `/` を含まない 2 要素）、(4) 各行で**インデントと `uses:` と owner/repo が同一**、
(5) **少なくとも 1 行で 40 hex が実際に変わっている**。変わってよいのは 40 hex と末尾のバージョン
注記だけで、末尾注記を許すのは dependabot が hex と一緒にここも書き換えることがあるため
（実例 `884f982` = `actions/setup-node` の `# v4` → `# v7.0.0`。ただし #591 の `3d6d3ea` は
`# v2` を据え置いており、どちらの形も通す必要がある）。**メジャー更新も免除する**——action の
入出力やランタイムが変わっても本文書が語る「どのジョブがあり、なぜ分けたか」は古くならないし、
action の挙動変化そのものは `ci` / `web` などの実ジョブが落ちることで検知される。

**理由**: 本文書が語るのは**ジョブ構成と分割の設計意図であって各 action の版ではない**ので、
ピンの hex が上がっても読み直す理由が無い。読み直す理由の無い変更で追従を強制すると、追従は
中身を見ない儀式に落ちる（#604 要件 (e) が測ろうとしている劣化そのもの）。owner/repo の同一性を
条件に入れるのは、action の差し替えはジョブの意味が変わり本文書の記述が古くなりうるから。
タグへの緩和（`@<40hex>` → `@v4`）はサプライチェーン対策の後退なので、片側が形式に合わず
自動的に内容変更になる——これも意図した挙動。例外 1b では吸収できない：
`is_metadata_only_change` は frontmatter に依存し、先頭行が `---` でない `.yml` では構造的に
効かない（ADR 0077 が `CLAUDE.md` を `sources` に入れない理由として既に記録している穴）。

**条件 1（パスの限定）と条件 3 の絞りは例外を意図した形に閉じ込めるためのもの。** 判定は行単位・
字面ベースで YAML 構造を見ないので、パスを絞らないと Markdown のコードフェンスに書いた `uses:` の
見本を書き換えただけでその文書の stale 検査が消える。owner/repo を 2 要素に限るのは、緩めると
再利用可能ワークフロー参照（`owner/repo/.github/workflows/x.yml@<sha>`）まで拾い、呼び先の
ジョブ構成ごと変わる更新を免除してしまうため。

**併せて `last_content_change` の走査窓（`limit=40`）の枯渇と履歴の尽きを区別する。** 呼び出し側は
`None` を warning に落として stale 判定をスキップする（fail-open）ので、「窓の中が全部除外対象
だった」だけで `None` を返すと**除外対象のコミットを積むほど検査が消える**。例外 1d で機械が量産する
コミットが除外対象になった以上これは現実的な経路なので、取れた件数が `limit` に達していたら次の
ページへ進む。戻り値は 3 通り: **SHA**（内容が最後に変わったコミット）/ **`None`**（**履歴が無い**
＝未コミット・履歴の尽き・shallow。従来どおり warning）/ **`ScanAborted`**（**走査を完遂できなかった**
＝ページ予算 `max_pages` / リネーム予算 `max_renames` / `git` コマンド自体の失敗（`git log` と
`git show --name-status` の両方）→ **error**）。
未完遂を `None` に混ぜると同じ fail-open が一段外側で再現するので分ける。番兵は素の文字列ではなく
型にする（str だと呼び出し側が SHA として `merge-base` へ渡し偽 STALE を出しうる）。予算は
**走査全体**で数える（リネームで取り直さない。パス単位にすると実際の上限が
`max_renames × max_pages` に膨らんで宣言と乖離し、病的な履歴では `adr` ジョブの timeout が先に来て
「打ち切りを error にする」意図が届かない）。`max_renames=N` で実際に辿れるリネームは N-1 段。
この修正は例外 1 / 1b にも効く。

**バイト列で比較する（改行コードも不正バイトも潰さない）。** ブロブ取得を `text=True` で行うと
universal newlines が `\r\n` を `\n` に潰し、**CRLF 変換とピン更新が同居したコミットが「ピン行だけの
差分」に見えて免除される**（`run:` ブロックの改行コードは shell の挙動を変えうる）。同様に不正 UTF-8 を
`errors="replace"` で復号すると**異なるバイト列が同じ U+FFFD に潰れて「行が一致」に見える**。取得と
行比較はバイト列で行い、復号は正規表現に当てる直前だけ・往復可能な `surrogateescape` で行う
（`splitlines()` は `\r` でも切るので `b"\n"` で分割する）。**この変更は例外 1b も締める**——
従来は「CRLF 変換＋メタデータのみ」が免除されていたが、今は内容変更になる（保守側なので許容）。

**却下した代替案**:

- **dependabot に `distilled_from_sha` を触らせる**: 自エコシステム外のファイルは編集しないので手段が無い。
- **人が拾う運用と割り切り手順を文書化する**: #607 を Actions 更新のたびに再演する。
  ADR 0073 の「人手の規律に委ねない」に反し、ピンの更新頻度を考えると恒久的な人件費になる。
- **`ci.yml` を `sources` から外す**: 構造的な赤は消えるが、ジョブ構成が変わったときの追従も消える。
  本文書は「主題そのものが対象ファイル」なので `sources` に入れる判断は ADR 0077 で維持済み。
  捨てるべきは検査ではなく粒度の粗さ。
- **`is_metadata_only_change` を汎用化して非 Markdown も扱う**: `.yml` に「メタデータ」の一般的な
  定義は無い。ピン行という具体形に限った述語のほうが例外の広がる余地が小さい。
- **hex のみの変更に限定する**: dependabot が末尾注記も書き換えることがあり（`884f982`）、
  その形の PR が例外に乗らない。
- **例外をパスで絞らず全 `sources` に適用する**: Markdown のフェンス内の見本を書き換えただけで
  その文書の stale 検査が消える。

**影響**:

- ピン更新 PR は人手ゼロで `adr` を通る。ピン更新と説明コメントの改訂が同居する PR（#607 の形）は
  従来どおり stale になり、これは意図どおりで `scripts/test-check-doc-classes.py` に対照ケースとして
  固定した。
- 例外が広すぎないことの担保はこのテストに依存する——ピン行以外の差分が 1 行でも混ざれば内容変更に
  落ちるので、「`ci.yml` を触った PR は何でも通る」への退行はテストで検出される。免除の境界
  （パスの限定・`.yaml` の許容・ワークフロー以外の `.yml` の除外・owner/repo の 2 要素・版注記の形・
  hex が実際に動いたこと・インデントの同一・行数の一致・CRLF 変換との同居・不正バイトの差し替えとの
  同居）は**それぞれ対照ケースで固定し、実装からガードを外すとそのケースが落ちることを mutation で
  全件確認してある**。
- **「ガードを消してもテストが緑」を避けるため、差し替え系の対照ケースはピン更新と同居させる形で
  書く**（hex を据え置くと別の条件で弾かれて狙ったガードを固定できない）。
- 走査窓のページングは例外 1 / 1b にも効く代わりに、除外対象が長く続く履歴では `git log` の呼び出しが
  走査全体で最大 `max_pages`（25）回まで増える（＝最大 1000 コミット。現実の `ci.yml`＝46 コミットでは
  1〜2 ページで終わる）。
- **既知の限界**（fail-closed 側なので実害はないが、調査の手間を省くために記録する）:
  CRLF で保存されたワークフローは行末の `\r` で正規表現が外れるので例外 1d が一切効かない
  （ADR 0081 の「既知の限界 (2)」）。
  もう 1 つ、**マージコミット内での純粋なリネームは免除が効かず偽の STALE になる**——
  combined diff はリネームを `RR` として出し `R100` にならないため（ADR 0084 決定 3）。
- **evil merge は不可視ではない**（ADR 0084。ADR 0081 の「既知の限界 (1)」の訂正）。
  `git show` はマージに対し**既定で combined diff（`--cc`）**を出し、`--cc` は
  「**全ての親と異なる**パス」を列挙する——これは evil merge（マージ自身だけが内容を変える形）の
  定義そのもの。片親と同じ内容になったマージは `--cc` が列挙しないが、**`git log` の既定の
  単純化もそのマージを列挙しない**（TREESAME な親を辿る）ので、走査がそこへ来ることが無い。
- **`status is None` の分岐はマージとは無関係**（ADR 0084 実測 4）。原因は `path_status` が
  name-status の**終点一致**しか見ないことで、**非マージのコミットで起きる**——
  リネーム元としてしか現れないコミット（`R100 <path> <新パス>` の終点は新パス）と、
  `sources` が非正規形（`./docs/...`）のとき。**この `continue` は load-bearing で、
  `return sha` に変えると純粋リネーム地点を内容変更と誤認して偽の STALE を出す。**
  到達回数の実測は実リポジトリ 0 回 / 回帰テスト **2 回**（非正規形 fixture の副作用 1 回 ＋
  下記の pin テストが意図的に踏む 1 回。pin テストを足す前は 1 回だった）。
  当初これを pin するテストが無かったので
  `test_rename_source_commit_is_skipped_not_attributed` を足した。
  - 実測（main `d46ace4`）: `git log -- <path>` が列挙したマージ × `sources` パス **7 組すべて**を
    `path_status` が検出（`MM` / `MA` / `AA`）、**不可視 0 組**。合成 fixture では 2 親の
    evil merge が `MM`、octopus（3 親）が `MMM` で検出できることも確認した。
  - evil merge は日常的に起きる。**PR ブランチが main を取り込んでコンフリクトを手で解消**すると、
    どちらの親にも無い内容がマージコミットに生まれる（`8ec61a18` が実例）。
  - **この `--cc` 依存は契約**。壊し方は 2 種類あり、**落ちるテストが違う**（実測）:
    - **マージが無出力になる変更**（`git diff-tree`（`-c` 無し）/ `--diff-merges=off`）
      → stale 検査に恒久的な穴が開く。`test_evil_merge_is_detected_as_content_change` が落ちる。
    - **第 1 親比較へ変える変更**（`--first-parent` / `-m`）→ **無出力にはならない**
      （第 1 親との差分が出るので evil merge は依然見える）。壊れるのは対象集合の方で、
      **片側だけ変えるとどちらも fail-open**（STALE が消える。実測）——`git show` 側だけなら
      マージ内リネームが `R100` に見えて免除が効き `test_rename_inside_merge_...` が、
      `git log` 側だけなら片親を採るマージの実変更コミットへ辿り着けず
      `test_merge_taking_one_side_...` が落ちる。**偽 STALE が出るのは両方を同時に変えたときだけ。**
  - **テストの組み立ての要点**（ここを外すと何も識別しない空テストになる）:
    - **両親の変更を免除対象（ピン更新のみ）で挟む**。そうしないとマージが不可視になっても
      親側の変更が STALE を出し、exit code が変わらない。
    - 対照群 `test_pin_only_merge_is_not_stale` は**解決に第 3 の hex を書く**。片親の hex を
      そのまま採ると対象パスについてその親と TREESAME になり、`git log` がマージを列挙せず
      `path_status` も免除分岐も一度も呼ばれない。
- **将来 dependabot の auto-merge を入れるなら、ピン差分の監査を別に持つ必要がある。** この例外は
  `adr` ジョブから「`ci.yml` が変わった」という自動シグナルを外す。owner/repo が同一でも、hex が
  同一リポジトリの未マージ PR の SHA を指せば任意コードが走る既知の攻撃面がある。現状
  `.github/dependabot.yml` に auto-merge の配線は無く人のレビューが残るので実害はないが、
  自動マージを入れる時点で「許可 owner/repo リスト」か「SHA の到達性検証」が前提になる。
- 同型の構造は `docs/api/openapi.json` を `sources` に持つ specifications 3 本にもあるが、生成物なので
  「実質変更なし」の差分が起きにくく実害が観測されていないため対象外。`sources` に `.md` 以外を
  入れている文書は 4 本だけ（**ワークフロー / コード**は本文書の `ci.yml` のみ、**生成物**は API 系
  3 本の `openapi.json` のみ）で、今後この種類が増えるときは種類ごとに「内容変更でない差分」の定義が
  要るかを検討する。

### build script が lock/checksum 外の資産を取ってこない（Swagger UI は vendored・ADR 0082）

`api-server` が依存する `utoipa-swagger-ui` の **build script は Swagger UI の zip をビルド時に
外部から取得していた**（`curl -sSL -o <path> <url>` の起動。**`--retry` も build script 側の再試行も
無く `build.rs:216` で 1 回失敗＝即 panic**）。上流が不調だと `docker-build (api)` が落ち、
2026-08-12（UTC）には配分定数と Python しか触っていない PR（#611）が 2 回連続で失敗した。
**壊れ方は 2 通り**で、1 回目は curl が exit 0 のまま壊れた本体を保存し（`-f` 無しなので HTTP
エラーを失敗と扱わない）`ZipArchive::new` が EOCD を見つけられず panic、2 回目は curl 自身が
非 0（56）で終了して `download_file` が Err を返した（こちらは `-f` の有無に関係なく落ちる）。

**「main はキャッシュで緑になるから上流障害を検知できない」という当初の分析は誤りだった。**
実測では、**関連パスを触った push は main でも毎回実ビルド**している（`5ae6466` は
`docker-build (api)` 3m45s でログに `Downloaded utoipa-swagger-ui v9.0.2` / `#14 DONE 177.2s`、
`ae8e33b` は 4m00s）。`#14 CACHED` が出たのは**すでにビルド済みの同一コミットを再実行したとき**
だけ（`eb9b9ce` の再実行・40s）。レイヤキャッシュはビルドコンテキストの内容でキー付けされ、
GHA のキャッシュはスコープが分離されていて main が PR ブランチ発の cache を読むこともないので、
**main / PR の非対称は存在せず上流が落ちれば main も落ちる**。`--mount=type=cache` の中身が
`type=gha` に載らないのは「RUN が走ったときに crate 取得を省けない」理由であって、非対称の
理由ではない。

**ただし「毎回」は関連パスを触った push に限る。** `docker-build` は自前の変更検出
（`Detect relevant changes`: `deployments/` / `src/` / `web/` / `Cargo.toml` / `Cargo.lock` /
`rust-toolchain.toml` / `ci.yml`）を持ち、これに触らない push は `run=false` で**スキップして
数秒で緑になる**。緑の `docker-build` を見るときは「実ビルドで緑」と「対象外でスキップ」を
所要時間で見分ける（前者は分単位、後者は 10 秒未満）——**この区別を落とすと同じ誤読が再生産される**。

**この誤読自体が外部取得のコスト**だった——一過性の失敗を前に、再実行の結果を根拠に「PR の
変更が原因では」と疑う方向へ 3 回の再実行と追試を費やした。

**決定**: `vendored` feature を有効にして外部取得をやめる。build script は
`CARGO_FEATURE_VENDORED` を最優先で分岐し、`utoipa-swagger-ui-vendored` の埋め込みバイト列を
使うので **build script は curl を起動せずネットワークにも出ない**（cargo 自体は crates.io を https で
叩くので CA 証明書は要る）。併せて `api.Dockerfile` の builder から `curl` を外す。`ca-certificates` は
**base の `rust:1.97-slim-bookworm` に同梱済み**で明示指定は冗長だが（`importer.Dockerfile` の builder は
入れずに通っている）、base が絞られたときの保険として残す。

**「取得をやめた」のではなく「検証とリトライのある経路へ載せ替えた」のが本質。** 旧経路は
`curl -sSL`（`-f` なし）で落としたバイト列を**ハッシュ検証なしに** unzip してバイナリへ埋め込む
（TOFU ですらない——TOFU は初回接触時に固定して以降の変化を検出するが、旧経路は何も固定して
いない）。だから HTTP エラーボディが zip として保存され上記の `InvalidArchive` が出た。新経路は
`Cargo.lock` の sha256 で検証され、取得失敗は cargo の transient retry に乗る。

**埋め込み版は従来のダウンロード版と同一**（実測）: `utoipa-swagger-ui-vendored` 0.1.2 は
`res/v5.17.14.zip` を同梱し `src/lib.rs` に "Swagger UI version: `5.17.14`" と明記していて、既定の
`SWAGGER_UI_DOWNLOAD_URL_DEFAULT` も同じ v5.17.14 タグを指す。**`/docs` の資産は 1 バイトも
変わらない。**

**理由**: ビルドの再現性を上流の稼働状況から切り離すのがいちばん安い（ADR 0026 で mupdf の版を
イメージタグで固定したのと同じ判断）。feature 1 つでコードは 1 行も変わらない。Swagger UI は
OpenAPI 仕様を描画する開発者向け UI なので、埋め込み版の版が変わっても paddock の API 定義
（`utoipa` 本体が生成）には影響しない。

**却下した代替案**:

- **リトライを入れる**: build script にリトライが無く外側で包むしかないうえ、上流が数十分落ちる
  ケースには効かない。「一過性かどうか」の判断を CI に埋め込むことになり切り分けコストも下がらない。
- **`cache` feature**: ダウンロード自体は消えない（OS のキャッシュに zip を残すだけ）。CI は毎回
  クリーンなランナーなので初回が必ず走る。
- **`SWAGGER_UI_DOWNLOAD_URL=file://...` で自リポの zip を指す**: 数 MB の zip を抱え、パスを
  Dockerfile と CI に配線する必要がある。vendored crate なら Cargo が同じことを管理する。
- **`docker-build` を required から外す**: **既に非必須**（ruleset の contexts は `ci` / `web` /
  `adr` / `predict-check` / `shellcheck` / `ocr-pdf` の 6 本）。かつ required の `ci` が
  `cargo clippy --locked --workspace --all-targets`（その後 `cargo test --locked --workspace
  --exclude pdf-ocr --exclude pdf-parser -- --test-threads=1`）で api-server をビルドするため
  同じダウンロードを踏む——
  `Swatinem/rust-cache` が miss すれば required check が上流障害で落ちる。**根治の動機はこちら。**
- **Swagger UI を dev feature に隔離して本番バイナリから外す**: 現状は外部露出が無いので同梱の害が
  実質ない。既定ビルドで `/docs` が消えるコストのほうが大きい（YAGNI）。**ただし「露出が無い」は
  (1) compose が api を `127.0.0.1:8080` に束縛していること、(2) `web.nginx.conf` が `/api/` しか
  proxy しないこと に依存する前提条件つきの結論**なので、崩れたら再検討する。`/docs` は `app.rs` の
  `/api` スコープの外なので、将来 `/api` に認証を入れても保護されない。
- **失敗時のメッセージだけ改善する**: 切り分けは楽になるが落ちる事実は残る。根治が feature 1 つで
  済むので緩和策を選ぶ理由が無い。

**影響**:

- `ci` / `docker-build` の両方でビルド時ダウンロードが消え、**上流の稼働状況に依存しなくなる**
  （実ビルドが走る run は main / PR を問わず落ちていたので、両方が救われる）。併せて
  「一過性の失敗を再実行で切り分ける」作業自体が不要になる。
- 依存が 1 本増える（`utoipa-swagger-ui-vendored` 0.1.2・依存ゼロ・build script なし・ライセンスは
  親と同じ `MIT OR Apache-2.0`）。**出荷されるバイナリのサイズは変わらない**（埋め込む dist が旧経路と
  同一なので）。増えるのは `target/` 内の build script バイナリ（+4.4 MB）で、crates.io からの +4.4 MB は
  同サイズの GitHub ダウンロードを置き換えるため cold build の取得量はほぼ相殺する。
- **CVE が出たときの更新経路が変わる**。vendored 有効時は `SWAGGER_UI_DOWNLOAD_URL` が**無警告で
  完全に無視される**ので「修正版の URL を差す」緊急回避は使えない（残る手は
  `SWAGGER_UI_OVERWRITE_FOLDER` か feature を一時的に外すこと）。dependabot が届くのも
  **`0.1.x` の範囲内だけ**——`utoipa-swagger-ui` の build-dependency 要件が `version = "0.1"` なので、
  上流が `0.2.0` で Swagger UI を上げても親が要件を上げるまで伝わらない。**検知手段はゼロではない**:
  `.github/workflows/audit.yml` の `cargo audit`（週次・`Cargo.lock` を照合）は新しい crate も射程に
  入るので RustSec に advisory が立てば拾える。**ただし Swagger UI 本体（JS）の CVE は RustSec に
  載らないので届かない**——これは vendored 化の前後で変わらない。
- **`vendored` が落ちる退行は機械で固定する**（`scripts/check-vendored-swagger.sh`。required の `ci`
  ジョブと pre-push）。feature が外れると build script は無警告でダウンロード分岐へ戻り、GitHub
  ランナーには curl があるので **required の `ci` は黙って外部取得を再開**する（落ちるのは非必須の
  `docker-build` だけ・原因の分かりにくいエラーで）。Dockerfile のコメントは人手の規律にすぎない。
  **主たる根拠は `Cargo.lock`**（optional な依存は feature で活性化されない限りロックに載らないので、
  在否がそのまま feature の効き方を表す・書式非依存・`--locked` で鮮度も担保）。宣言側も見るのは
  「宣言を消したがロックを再生成していない」状態を拾うためで、**照合は単一行に限定しない**
  （`features` の複数行整形は正当なので、単一行 grep だと整形だけで落ちる＝偽陽性になる）。
- **`/docs` の配信そのものも機械で固定する**（`src/apps/api-server/tests/docs_ui.rs`・#616）。
  feature 検査（`check-vendored-swagger.sh`）が見るのは**宣言**で、配信側は別の退行になる。
  **ただし「資産の取り込みに失敗する」ケースはここには来ない**——zip の展開失敗は上流の build script が
  `expect` で panic するので**コンパイル時に落ちる**。この検査が押さえるのは **(a) 上流の版が上がった
  ときの資産名・構造のドリフト**、**(b) `SwaggerUi` の配線ミス**（マウント先・spec URL・別 `ApiDoc` の
  混線）、**(c) 描画元に外部オリジンが混ざる逆戻り**（`vendored` feature の脱落自体は配信 HTML が
  同一なので検知できない——そちらは `check-vendored-swagger.sh` の担当）。いずれも
  資産が「在る」まま UI だけが壊れる。だから **200 が返るだけでは足りない**:
  `index.html` が**参照する資産名**と、その資産が**実体を伴って配信されるか**を両側から見る
  （片側だけだと「index.html だけ新しくなって資産名が変わった」を取り逃がす）。加えて配信された
  spec が `ApiDoc::openapi()` と一致することも見る——生成側の検査（`openapi.rs`）は配信物を見ないので、
  別の `ApiDoc` が `SwaggerUi::url` に配線される退行はここでしか捕まらない。
  個々のアサーションは `docs_ui.rs` の doc コメントが正（ここに列挙すると実装とドリフトする）。
  **このテストは「docs を無認証で配信してよい」を要件として承認したものではない**——`/api` の外に
  あるという現状の追認にすぎず、保護を入れる判断とは独立（`app::configure_routes` の doc 参照）。
  **案内する URL は `/docs/`**——テイルマッチなので末尾スラッシュ無しの `/docs` は 404（扱いは #619）。
  本文中の他の `/docs` 表記は配信経路を指す一般参照で、叩く URL ではない。
  **手動のブラウザテストに残るのは JS 実行後の描画結果とコンソールエラーだけ**
  （[api-docs-swagger-ui.md](../../tests/browser-test-cases/api-docs-swagger-ui.md) の TC-01）。
- **`-vv` のログの読み方**: `SWAGGER_UI_DOWNLOAD_URL: <url>` は **vendored でも印字される**ので
  ダウンロードの証拠にならない。実際に取得したかは `using vendored Swagger UI`（vendored 経路）と
  `start download to`（ダウンロード経路）のどちらが出るかで見る。
- builder から `curl` が消えたので、将来ビルド時に curl が必要な依存を足すときは戻す。
- `docker-build` が非必須である事実は変えない——required にするかは別の判断。

### `test_extract.rs`（tesseract OCR）は `#[ignore]` のまま

tesseract の版・言語データ差に依存するテストで、決定論を確保できない。CI 標準に載せると flaky 化する。
「OCR 統合テストを CI で実走する」という要件は、**mupdf 依存の render/parse を実走対象に戻す**ことで
満たしている。

### JRA 取得失敗は skip に倒す

サンプル PDF をリポジトリに含めない設計なので、統合テストは実行時に JRA から取得する。取得できない
run では**アサーション未実行のまま緑になる**（`#[ignore]` ではなく早期 return）。JRA の一時不通で CI を
赤くしないための意図的な選択だが、**mupdf 依存の解析回帰は「取得に成功した run」でのみ実証される**
という穴でもある。ユニットテストは常時走るのでカバレッジの土台はある。恒常的な実走保証が要るなら
サンプルの別保管（暗号化アーティファクト等）を別途検討する。

## 既知の版ずれ

| 環境 | mupdf | 備考 |
|---|---|---|
| dev（macOS） | 1.27.2 | 両版とも現行アサーションを満たすことは確認済み |
| CI（`ocr-pdf`） | 1.25.1 | `debian:trixie-slim` |
| importer runtime | 1.21 | `importer.Dockerfile` が debian **bookworm**。`MutoolParser` 単体では **0 レースになる版** |

importer は OCR ハイブリッド経路なので単体版と挙動が異なる。bookworm → trixie の引き上げは
ADR 0026 のスコープ外として記録されたまま**未確認**。将来 mupdf の出力が変われば dev と CI で
割れうるので、そのときはイメージタグ更新かアサーション調整で対応する。

## 関連

- ADR: 0026 OCR/PDF 統合テストを CI で実走（mupdf 版固定） /
  0073 ADR 統合と文書クラス・機械検査 /
  0081 ピン更新だけの差分は内容変更と見なさない /
  0082 Swagger UI を vendored にしてビルド時ダウンロードを消す
- 必須チェックの ruleset は #461（ジョブ ID `adr` は必須チェック名なので改名しない）
- pre-push は CI 相当の高速チェックを手元で再現する（`scripts/git-hooks/pre-push`。配線は
  `scripts/install-git-hooks.sh` で clone ごとに一度）

---

## 決定ログ

<!-- この節は append-only です。既存エントリの変更・削除は CI が検出します。 -->

### ADR 0026: OCR/PDF 統合テストを CI で実走する (mupdf バージョン固定) (Issue #172) (2026-06-19) — 承認済み

#### コンテキスト

#160（PR #169）の CI 新設時、`pdf-ocr` / `pdf-parser` の統合テスト（`tests/test_render.rs` / `tests/test_parse.rs`）は CI で実走できず、crate ごと `--exclude` した（OCR 非依存のユニットテストのみ `--lib` で実走）。Issue #172 で CI 再現可能化を図る。

調査（debian / ubuntu 各イメージで `samples/2026-3nakayama6.pdf` を実走）で、**失敗の主因は tesseract ではなく `mutool`（mupdf）のバージョン**だと判明した:

- `pdf-parser` の `MutoolParser` は `mutool draw -F text` / `-F stext.json`（mupdf）**のみ**を使い、tesseract は呼ばない。`pdf-ocr` の `render_pdf_to_pngs` も `mutool draw -F png` のみ。
- tesseract を実際に使うのは `pdf-ocr` の `OcrExtractor::extract`（`tests/test_extract.rs`）だが、これは遅いため既に `#[ignore]`。
- サンプル PDF の解析結果（12 レース・騎手名・調教師名・距離等の具体値アサーション）は mupdf のテキスト抽出出力に依存し、版が古いと **0 レース**になる:

| mupdf | 環境 | 結果 |
|---|---|---|
| 1.21.1 | debian bookworm（= importer runtime） | 0 レース・FAIL |
| 1.23.10 | ubuntu 24.04（= `ubuntu-latest`） | 0 レース・FAIL |
| 1.25.1 | debian trixie / ubuntu 25.04 | 全アサーション PASS |
| 1.27.2 | macOS dev（homebrew） | 全アサーション PASS |

すなわちパーサは mupdf **≥ 1.25** で互換。`ubuntu-latest` は apt で 1.23 しか入らないため、apt インストールだけでは再現できない（PR #169 で mupdf-tools を入れても失敗し除外した経緯と一致）。

#### 決定

- `pdf-ocr` / `pdf-parser` の統合テストを、**mupdf 1.25 を持つ `debian:trixie-slim` コンテナ上の専用 CI ジョブ**（`ocr-pdf`）で実走する。
  - ジョブは `runs-on: ubuntu-latest` + `container: debian:trixie-slim`。apt で `mupdf-tools`（1.25.1）/ `tesseract-ocr` / `tesseract-ocr-jpn` / `build-essential`（rustls の ring が C/asm をビルドする）/ `git curl ca-certificates`（checkout・rustup ブートストラップ）を入れる。ureq は rustls（純 Rust TLS）で sqlx(native-tls) は閉包外のため `libssl-dev`/`pkg-config` は不要。
  - Rust toolchain は本体ジョブと同じ 1.96.0（`rust-toolchain.toml`）。`Swatinem/rust-cache` でキャッシュ。
  - `cargo test --locked -p pdf-ocr -p pdf-parser -- --test-threads=1`（lib + 統合）を実走する。Postgres は不要（これらは DB を触らない）。`--test-threads=1` は複数テストバイナリの並行 JRA 取得を避け出力を決定的にするため。
  - mupdf の版ドリフト対策として、テスト前に **`mutool` のバージョンが 1.25 以上であることを assert する gate ステップ**を置く。下限未満ならサイレントに 0 レース化させず明示的に fail させる。
- **`test_extract.rs`（tesseract OCR・低速）は `#[ignore]` のまま**とする。tesseract の版・言語データ差に依存し決定論性が低いため、CI 標準実行には載せない（必要時に `--ignored` で手動実行）。
- 本体 `ci` ジョブは従来どおり `--exclude pdf-ocr --exclude pdf-parser` を維持し、これまで本体に置いていた「OCR クレートの `--lib` のみ」ステップは新ジョブへ移管して重複を解消する。
- サンプル PDF は JRA 著作物で repo に含めない（gitignore）。CI ではフィクスチャが JRA 公式から取得を試み、取得できれば統合テストが実走、取得不可なら graceful skip（ユニットテストは PDF 不要で常時実走）。

#### 理由

- **版固定がコンテナで最も再現可能**: `ubuntu-latest` の apt mupdf（1.23）は不足、ソースビルドは CI を重く・脆くする。`debian:trixie-slim` は apt 一発で 1.25 が入り、イメージタグで版を固定できる。
- **専用ジョブで分離**: 本体ジョブは Postgres サービス + `localhost` 前提。全体を container 化すると service ネットワーク（`localhost`→サービス名）や DB 接続を作り直す必要があり影響が広い。pdf テストは DB 不要なので別ジョブに切る方が安全・並列で速い。
- **`test_extract` を ignore 維持**: 真に tesseract 版依存なのはこのテストだけ。決定論を確保できないものを CI 標準に載せると flaky 化するため、対象外を継続する（Issue 要件の「OCR 統合テストを CI 実走」は mupdf 依存の render/parse を実走対象に戻すことで満たす）。
- **取得失敗を skip に倒す**: フィクスチャ既存設計（PDF 取得不可なら `None`→早期 return）を踏襲し、JRA 一時不通で CI を赤くしない。ユニットテストは常時カバレッジを担保する。
- **イメージは tag 参照（digest ピンしない）**: 外部 action は SHA ピンするが、コンテナイメージは本体  job の `postgres:17-alpine` と同様に tag 参照とする（OS イメージはセキュリティ更新を取り込みたく、digest 固定は陳腐化・手動更新の負担が大きい）。版ドリフトの実害（mupdf が下限割れ）は上記 assert gate で検知する。

#### 影響

- CI に新ジョブ `ocr-pdf` が増える（コンテナ pull + 依存 apt + ビルド）。`rust-cache` 前提で 2 回目以降は短縮。
- dev（macOS mupdf 1.27.2）と CI（trixie mupdf 1.25.1）で mupdf 版が異なる。両版とも現アサーションを満たすことは確認済みだが、将来 mupdf 出力が変わればどちらかで割れうる。その場合はコンテナのイメージタグ更新かアサーション調整で対応する。
- importer runtime（`importer.Dockerfile`）は debian **bookworm**（mupdf 1.21）であり、`MutoolParser` 単体では 0 レースになる版。importer の解析経路（OCR ハイブリッド）への影響確認・bookworm→trixie 引き上げは本 Issue のスコープ外（別途要確認）として記録する。
- **JRA 取得不可日のカバレッジ低下**: サンプル PDF を repo に含めない設計上、JRA から取得できない run では統合テストがアサーション未実行のまま緑になる（`#[ignore]` ではなく早期 return のため）。ユニットテストは常時走るが、mupdf 依存の解析回帰は「取得成功した run」でのみ実証される。恒常的な実走保証が必要になれば、サンプルの別保管（暗号化アーティファクト等）を別 Issue で検討する。本 PR では Actions 実 run で 12 レース解析の pass ログを確認して実走を実証する。
- ADR は連番末尾 `0026` で採番（採番当時 `0022` が重複していたため。重複は後に是正済み＝後発の `jra-fetcher 集約` を ADR `0029` にリナンバー、2026-06-20）。

### ADR 0073: ADR を一次資料層へ統合し、D01〜D24 文書クラスと機械検査を導入する (2026-08-09) — 承認済み

#### ステータス

承認済み。決定 1（ADR の一次資料層への統合）は #577 で実装。決定 2〜4（stale 機械検査・D01〜D24 文書クラス・プロダクト目標）は **#579** で追跡する。

#### コンテキスト

paddock は HVE（dahatake/HypervelocityEngineering, MIT）の 3 層蒸留モデル（docs-original → qa → knowledge）と mdq 検索を既に取り込んでいる。しかし実測すると、層の切り方と運用の両方に構造的な問題があった。

##### 層の重複が実害を出している

#568 の 4 点セット（docs-original / qa / knowledge / ADR、合計 515 行）を全文照合した結果（測定は #576 のマージ前に同 PR のブランチ上で実施。#576 マージ後は `docs/knowledge/monitor-loop-sleep-resilience.md` と ADR 0072 として本リポジトリで参照できる）:

- `docs/knowledge/monitor-loop-sleep-resilience.md` の本文 103 行のうち **88 行（85%）が ADR 0072 と 1:1 対応**し、knowledge が追加した決定は **0 件**。固有の価値は運用者向けの読み方 6 行に集約されていた。
- 「5 秒刻みの根拠（DarkWake 累計 28 秒）」「単調時計で所要を測る理由」「JST 変換を持ち込まない理由」は、いずれも **qa / knowledge / ADR の 3 箇所に語順までほぼ同一**で存在した。
- `docs/qa/QA-analyze-384.md` の Q2/Q3 は回答文が **約 90% 逐語**で knowledge へ移送され、knowledge が足した固有情報は 1 行だった。
- `docs/docs-original/` 4 本はすべて **GitHub Issue 本文の 25〜38% を逐語コピー**していた。しかも #384 は「別 issue」→「#379・実装済」に改変、#389 は「現状」章を削除、#401 は「要件」章 4 項目を削除しており、**原本として機能していない**。
- 同一の実測（`name=カップ` → starts=0）が **5 ファイル**に重複して存在した。

##### 蒸留層の権威が逆転している

`docs/qa/QA-setup-boilerplate-410.md` には「【追記・#453 で覆る】`NoopParser` / `NoopFetcher` スタブは削除された」とある。ところが蒸留先の `docs/knowledge/app-bootstrap.md` は `status: Confirmed` のまま `NoopParser` の注入を推奨し続けている。コードを実測すると `NoopParser` はソースツリーに **1 件も存在しない**。

「qa は生ファイル、knowledge が確定知」という規約（[docs/qa/README.md](../qa/README.md)）と実態が逆転しており、**knowledge を信じると存在しない API を書く**。`docs/knowledge/README.md` の第 6 ステップ「sources 追従」は規約として存在するが、機械検査が無いため守られていない。

##### 蒸留が日常開発に乗っていない

knowledge / specifications 22 本の `updated` は全件 2026-07-16〜07-30 に集中し、`distilled_from_sha` も 11 本が同一 SHA。一括整備で作られたきり止まっている。`status` は全 22 本が `Confirmed` で、`Tentative` / `Conflict` の運用実績が無い。

##### 分類軸が無い

`docs/` にプロダクトの目標・成功条件・非目標を書いた文書は **0 件**（全 106 本を検索して該当なし）。方向性は ADR 71 本を読み解くことでしか復元できない。また文書を横断的に分類する軸が無く、`docs/adr` / `docs/specifications` / `docs/knowledge` というディレクトリの区別しか無かった。

#### 決定

##### 1. ADR を `docs/docs-original/` へ物理移動し、一次資料層に統合する

ADR 71 本（0001〜0071）を `docs/adr/` から `docs/docs-original/` へ移す。ディレクトリ `docs/adr/` は廃止する。

命名で 2 系統を分離する。**この規約が ADR 番号重複検出の判定根拠**になる。

| 種別 | 命名 | 例 |
|---|---|---|
| ADR | **0 埋め 4 桁**（`0001`〜`0999`。上限は 0999 で、超えるときは判定と規約を併せて見直す） | `0055-ev-layer-separation-circular-break.md` |
| issue 由来の一次資料 | **issue 番号・0 埋めしない** | `382-live-server-now.md` |

`scripts/check-adr-numbers.sh` は走査先を `docs/docs-original` に変え、ファイル名が `0` + 3 桁（`^0[0-9]{3}`）で始まるかで ADR を分離する。非 ADR は**黙ってスキップ**する（警告に載せると本来見るべき重複検出が埋もれる）。

そのうえで、**「黙って緑になる」経路を 4 つ塞ぐ**。fail-closed の判定は壊れても本番データが正常なら気づけないため、使い捨て fixture による回帰テスト（`scripts/test-check-adr-numbers.sh`）で各分岐の終了コードと文言を固定し、CI で本番検査より先に走らせる。

1. **0 埋めを忘れた ADR**（`74-foo.md`）— 主判定の網から漏れて重複検出を無効化する。判定は H1 の書式ではなく**本文構造**（`## ステータス` と `## 決定` が、コードフェンスの外の行頭に同時存在）で行う。H1 は `# ADR 0001: …` と `# 0071. …` の 2 系統に割れているうえ、番号の桁数でマッチさせると 2 桁 ADR を取りこぼし、逆に一次資料の H1 が `# 401: …` 形式になると誤検知して CI を全停止させる。フェンスと行頭を絞るのは、docs-original が issue 本文や外部資料を逐語転記する層で、引用やコードフェンスの中に ADR 雛形が現れうるため。実測で ADR 72/72 がこの構造を満たし、一次資料 4/4 が満たさない。
2. **ADR 0 件** — 従来の `exit 0`（fail-open）から `exit 1` へ。
3. **旧 `docs/adr/` に ADR が置かれている** — 決定 1 の「統合前に分岐した PR」対策（下記「影響」参照）。判定はディレクトリ存在ではなく中の `*.md` の有無で行う（git は空ディレクトリを追跡しないので、空の `docs/adr` はローカル残骸でしか現れず、そこで落とすと pre-push が恒久的に詰まるだけ）。
4. **サブディレクトリへの配置** — 走査は直下限定なので、`docs/docs-original/adr/0001-x.md` のような階層を切られると重複検出・採番の両方から不可視になる。

いずれの致命チェックも **`check` だけでなく `next`（採番を配る経路）にも効かせる**。走査が壊れた状態で `next` が番号を返すと、既存 ADR と衝突する採番をそのまま配ってしまうため。番号の重複判定には「先頭の連続数字」をそのままキーに使い、規約外の桁数（`00401-*.md`）でも重複が漏れないようにする。

##### 2. ADR の内容は knowledge へ全部写す。同期は機械検査で担保する

読む入口を knowledge に一本化する。ADR の決定・理由・却下案・影響を knowledge に写し、ADR 自体は一次資料として不変のまま残す。

重複を許す代わりに、`sources` に列挙されたファイルの最終コミットが `distilled_from_sha` の子孫かを機械検査する（`git merge-base --is-ancestor`）。CI と pre-push の両方に配線する。

検査には**例外を 2 つ**設ける（詳細と理由は [docs/knowledge/README.md](../knowledge/README.md) の「sources 追従」）。素朴に実装すると本 ADR の移動自体で全件が誤検知するので、実装時に必ず織り込む。

1. **rename-only のコミット（内容差分ゼロ）は比較対象から除外する**。`git log --follow` では吸収できない——`--follow` はリネームより前へ履歴を遡らせるだけで、「最終コミット」がリネームコミットになる事実は変わらないため、そのままだと本 ADR で `sources` パスを書き換えた 20 本すべてが stale 判定になる。
2. **`status: Conflict` の宣言だけを足したときは `distilled_from_sha` を据え置く**（`updated` のみ bump）。乖離に気づいた記録であって再蒸留ではないため。`Confirmed` に戻すとき（＝実際に差分マージしたとき）に現 HEAD へ進める。

**順序は「機械検査の配線が先、写しは後」**。写した量に比例して stale 面積が増えるのが本 ADR の出発点（`app-bootstrap.md` の事故）なので、担保のないまま 72 本ぶんの写しを始めると、解こうとしている問題を自分で拡大することになる。移行が完了するまでは knowledge だけでなく ADR 原本も読む運用とし、その旨を `CLAUDE.md` と `docs/knowledge/README.md` に移行中ブロックとして明示する。

##### 3. HVE の D01〜D21 文書クラスを採用し、D22〜D24 を追加する

D01〜D21 は番号・名称を変えず採用する（HVE との語彙互換を保ち、将来の追加移植の摩擦を避ける）。paddock 資産の実測で、**99 本中 54 本（54.5%）が D01〜D21 のどこにも入らない**ことが分かったため、3 クラスを追加する。

| クラス | 内容 | 該当本数 |
|---|---|---|
| D22 | 予測モデル・特徴量仕様 | 31 |
| D23 | 買い方・資金配分ルール | 18 |
| D24 | 実験・検証記録／棄却証跡 | 5 + `-rejected` ADR 24 本 |

**D03 / D12 / D13 / D14 / D20 は「N/A（単独開発・ローカル運用）」を 1 行宣言して閉じる**。空文書は作らない。

物理表現は frontmatter `doc_class`（正本）とし、`tags` に同値をミラーする。ファイル名は変えない。

##### 4. プロダクト目標文書（D01）を新設する

「数値で競馬を見る」「買い方を楽しく売れる形で提示する」を目標として明文化し、成功条件（ROI ≥ 100% ゲート・精度実績）と非目標（棄却 ADR 24 本から復元）を 1 枚にまとめる。**収益化の具体（価格・販路）は書かない**。

#### 理由

- **ADR と一次資料は「一度置いたら書き換えない（RO）」という性質が同じ**。同じ層に置くことで、`ADR → knowledge` の写しが例外的な重複ではなく規約どおりの蒸留になる。層の数を減らさずに、責務の説明を一本化できる。
- **移動コストが小さいことを実測で確認した**。`docs/adr` と `docs/docs-original` は同じ階層深さ（`docs/` 直下）なので、相対リンクはどの参照元からも「`adr` → `docs-original`」の 1 語置換で閉じる。ADR 本文が持つ兄弟相対リンク 8 件・`../specifications/` 6 件・`../images/` 1 件・`../../deployments/` 2 件は**無変更で解決する**。ファイル名衝突も 0 件だった。
- **「全部写す」を選ぶ以上、機械検査は必須**。写した量に比例して stale 面積が増える。`app-bootstrap.md` の `NoopParser` 事故は 1 件で済んだが、71 ADR ぶんに広げれば人手の規律では守れない。ADR 番号の重複検出（#254）と同じ判断——人手で再発が防げないものは機械で弾く。
- **D01〜D21 をそのまま採るのは HVE 互換のため**。番号を変えると将来 HVE の資産（skill・prompt）を追加移植するときに読み替えが要る。空クラスが 12 個出るが、うち 5 個は N/A 宣言で閉じ、残り 4 個（D01 成功条件・D07 用語集・D15 SLO/Runbook・D21 CI/CD の文書化）は**真の欠落**で、埋めること自体が D 体系採用の実利になる。
- **企業分析・業界分析が無くても上流は書ける**。HVE の ARD ワークフローは `target_business` 指定時に Step 1（事業分野候補列挙）を skip する設計を持っており、対象が決まっている個人プロダクトは正規ルートでその経路に乗る。paddock の「業界分析」に相当する市場（オッズ）の性質分析は、ADR 0027 / 0055 / 0058 / 0059 / 0067 として**既に蓄積済み**で、足りないのは上位のゴール文書 1 枚だった。

#### 却下した代替案

- **ADR を `docs/adr/` に残し、位置づけの宣言だけ変える**。リンク破壊もツール改修もゼロで済み、mdq は `docs/adr` を索引済みなので検索体験も変わらない。実利/コスト比では最も良いが、ディレクトリ構成が 3 層モデルと一致しないままになる。**利用者の判断で物理移動を採用した**。
- **`docs/docs-original/adr/` へサブディレクトリとして移動**。生ログと ADR の混在を避けられるが、パス一斉改修のコストは同じで、階層深さが変わるぶん ADR 本文の相対リンク 17 件も書き換えが要る（フラット移動なら不要）。
- **knowledge を「複数 ADR を横断するときだけ作る」に限定する**（＝ ADR 1 本に knowledge を作らない）。#568 の 85% 重複は消えるが、「今どうなっているか」を知るのに ADR と knowledge を往復することになる。読む入口の一本化を優先して却下した。
- **D22〜D24 を作らず D06（業務ルール・判定表）/ D17（UAT）へ押し込む**。HVE と完全同一の 21 クラスを維持できるが、D06 の必須項目「判定表・override 承認者・発効/失効日・根拠規程」が予測モデル 31 本すべてで UNKNOWN になる。統計モデルに承認者も規程根拠も存在しない。
- **D クラスをファイル名プレフィックス（`D08-*.md`）で表現する**（HVE 流）。`mdq --paths` で絞れる利点があるが、22 本のリネームで `sources` 参照が再度壊れる。`doc_class` + `tags` ミラーで同等の絞り込みが得られるため却下。

#### 影響

- **移動**: ADR 71 本が `docs/adr/` → `docs/docs-original/`。`docs/adr/` は消滅。
- **変更（機械置換 187 箇所 / 33 ファイル）**: frontmatter `sources` のパス、本文の相対リンク、規約文。`git grep` / `git ls-files` に限定して実施した（`.claude/worktrees/` の並走 worktree 3 本がそれぞれ完全な `docs/adr/` を持つため、`grep -r` では別ブランチの作業コピーを破壊する）。
- **変更**: `scripts/check-adr-numbers.sh`（走査先・ADR 分離・fail-closed 化）、`mdq.toml`（`docs/adr` root を削除。実体が消えているので `iter_markdown` の `base.exists()` で skip され残しても無害だが、死んだ設定は残さない）。
- **追加**: `scripts/test-check-adr-numbers.sh`（fail-closed 分岐の回帰テスト）と CI `adr` ジョブへの配線。本番検査より**前**に走らせる（本番検査が落ちたとき、ADR が本当に重複しているのか判定器が壊れているのかを切り分けられるようにするため）。
- **不変**: ADR の採番方式、CI ジョブ ID `adr`（ruleset #461 の必須チェックなので改名しない）。ADR 本文は 71 本中 **70 本がバイト同一**で移動した。唯一の例外は `0062-workout-cyokyo-feature-rejected.md` で、本文のコードブロック内に自ディレクトリの絶対パス表記（`docs/adr/0061`）があったため 1 行だけ機械置換の対象になっている。「ADR は改変しない」規約に対する意図的な例外——旧パスのまま残すとリンクではないにせよ存在しないディレクトリを指し続けるため、パス表記の正確性を優先した。
- **運用**: 新しい ADR は `docs/docs-original/0NNN-*.md` に置く（採番は `scripts/check-adr-numbers.sh next`）。issue 由来の一次資料は 0 埋めしない。mdq で ADR だけに絞るなら `--paths "docs/docs-original/0*"`。既存の索引を持つ環境は一度だけ `rm -rf .mdq && scripts/mdq index` で作り直す（prune は roots 配下しか消さないため、旧 `docs/adr/*` のチャンクが居残る）。
- **統合前に分岐した PR への影響**: 本統合より前に分岐した PR が `docs/adr/` に新しい ADR を足していると、パスが異なるため git は競合を報告せず**どちらの順でマージしても無言で通る**。結果 `docs/adr/` が復活し、その ADR は `check-adr-numbers.sh` の走査先（`docs/docs-original`）から見えず番号重複検出が穴あきになる。これを防ぐため、**`docs/adr/` 配下に `*.md` が置かれていることを致命扱いにするガード**を同スクリプトに入れた（該当 PR がマージされた時点で CI が落ち、対処手順を出力する）。ディレクトリの存在ではなく中身で判定するのは、git が空ディレクトリを追跡しないため——空の `docs/adr` は `.DS_Store` 等が居るローカル環境でしか現れず、そこで落としても防ぎたい事故は何も防げずに pre-push が詰まるだけになる。

実例: #576 が `docs/adr/0072-monitor-loop-wall-clock-sleep-resilience.md` を旧パスに追加した状態で先にマージされた。本統合を rebase したところ git が `file location` conflict として検出し（「rename されたディレクトリ内に追加された」）、0072 も本統合の移動対象に含めて解決した。あわせて `docs/knowledge/monitor-loop-sleep-resilience.md` の `sources` と `deployments/launchd/README.md` のリンクを新パスへ追従させている。**git が conflict として拾えたのは rename を含むコミットを rebase したからで、マージ順序によっては無言で通る**——ガードはその場合の保険として残す。
- **後続（追跡: [#579](https://github.com/taito-station/paddock/issues/579)）**: stale 機械検査と D クラス体系（PR2）、プロダクト目標と REQ-ID 規約（PR3）、質問票 skill の汎用改修（PR4・dotclaude 側）。既存 ADR の REQ-ID 遡及紐付けと knowledge への全写しの実施は段階的に進める。**写しは機械検査の配線後**（順序は決定 2 参照）。
- 関連: #254（ADR 番号重複検出）／ADR 0064（second source を戒める）／[docs/knowledge/README.md](../knowledge/README.md)（蒸留規約の正）。

#### 再現方法

```sh
# ADR の重複検出（72 件・次番号）
bash scripts/check-adr-numbers.sh

# fail-closed の全分岐（0 埋め忘れ / 2 桁 / ADR 0 件 / 旧 docs/adr / サブディレクトリ /
# 引用・コードフェンスでの誤検知なし / 引数処理）は回帰テストで固定してある。
# 手で fixture を作らずにこれを走らせるのが正（手動 mv は untracked ファイルが残って
# リポジトリを壊れた状態にしやすい）。
bash scripts/test-check-adr-numbers.sh

# 旧パスへの「参照」が 0 件であること。
# docs/adr という文字列自体は「旧 docs/adr から統合した」という履歴参照や、復活検出ガードの
# エラーメッセージとして意図的に残るため、素の grep 件数は不変条件にしない。
# 参照として意味を持つ 3 形態がゼロであることを見る。
git grep -n '\.\./adr/' -- .                  # 兄弟相対リンク           → 0 行
git grep -nE '\]\(.*docs/adr/' -- .           # Markdown リンクの宛先    → 0 行
git grep -nE '^  - docs/adr/' -- docs         # frontmatter sources     → 0 行

# mdq 再索引と ADR 絞り込み
scripts/mdq index
scripts/mdq search --q "EV 層分離" --paths "docs/docs-original/0*" --top-k 3
```

### ADR 0081: `uses:` のピン留め SHA 更新だけの差分は「内容変更」と見なさない (2026-08-13) — 承認済み

#### ステータス

承認済み（[#612](https://github.com/taito-station/paddock/pull/612) で実装）。
対象 Issue: [#606](https://github.com/taito-station/paddock/issues/606)。
関連: ADR 0073（機械検査の導入）/
ADR 0077（`sources` の範囲。frontmatter を持たない
ファイルでは `is_metadata_only_change` が構造的に効かないことを記録済み）。

#### コンテキスト

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

#### 決定

**`uses:` 行のピン留め SHA 更新だけの差分を、stale 検査の「内容変更」から外す**
（[docs/knowledge/README.md](../knowledge/README.md) の**例外 1d**）。

判定は `scripts/check-doc-classes.py` の `is_pin_only_change(sha, path)` が行い、
`last_content_change` が例外 1 / 1b と同じ位置で呼ぶ（該当コミットを飛ばして実質の変更点まで遡る）。
真になる条件は次のすべて。

1. **対象パスがワークフロー**（`.github/workflows/*.yml` / `*.yaml`）である
2. 変更前後で**行数が同じ**（行の増減はジョブ・ステップの追加削除なので内容変更）
3. 変更のある行が **1 行以上**あり、その**すべて**が
   `^(\s*(?:-\s+)?uses:\s+)([^@\s/]+/[^@\s/]+)@([0-9a-fA-F]{40})([ \t]+#[ \t]*v?[0-9][0-9A-Za-z._+-]*)?$` にマッチする
   （`#` の前の空白を必須にするのは、`@<40hex>#v4` は YAML ではコメントにならず ref の一部だから。
   注記を `[ \t]` と `[0-9]` で書くのは、`\s` と `\d` も Unicode 対応なので
   `# v4だがこの版は使わない` のような散文付きや `# v٤`（アラビア数字）・NBSP 区切りが
   「版注記」として通ってしまうから）
4. その各行で、**インデントと `uses:`（group 1）と owner/repo（group 2）が変更前後で同一**
5. 変更のある行の**少なくとも 1 行で 40 hex（group 3）が実際に変わっている**

変わってよいのは **40 hex（group 3）と末尾のバージョン注記（group 4）だけ**。末尾注記を許すのは、
dependabot が hex と一緒に注記も書き換えることがあるため——実例は `884f982`
（`actions/setup-node` の `# v4` → `# v7.0.0`）。ただし常に書き換わるわけではなく、
#591 の `3d6d3ea`（`Swatinem/rust-cache`）は `# v2` を据え置いている。どちらの形も通す必要がある。

**条件 1（パスの限定）と条件 3 の絞りは、例外を「意図した形」に閉じ込めるためのもの。**
判定は行単位・字面ベースで YAML 構造を見ないので、パスを絞らないと **Markdown のコードフェンスに
書いた `uses:` の見本を書き換えただけでその文書の stale 検査が消える**。owner/repo を `/` を含まない
2 要素に限るのは、緩めると再利用可能ワークフロー参照
（`owner/repo/.github/workflows/x.yml@<sha>`）まで拾い、呼び先のジョブ構成ごと変わる更新を免除して
しまうため。末尾注記を版の形に限るのは、任意コメントを許すと注記を無関係な散文へ差し替えた変更まで
免除されるため。条件 5 が無いと、hex が 1 文字も動いていない注記だけの書き換えも通ってしまう
（「ピン留め SHA 更新のみ」という例外の条件から外れる）。

**併せて、`last_content_change` の走査窓（`limit=40`）の枯渇と履歴の尽きを区別する。**
呼び出し側は `None` を warning に落として stale 判定をスキップする（fail-open）ので、
「窓の中が全部除外対象だった」だけで `None` を返すと **除外対象のコミットを積むほど検査が消える**。
例外 1d で機械が量産するコミットが除外対象になった以上これは現実的な経路なので、取れた件数が
`limit` に達していたら次のページへ進む。戻り値は 3 通りにする。

- SHA: 内容が最後に変わったコミット
- `None`: **履歴が無い**（未コミット・履歴が尽きた・shallow）→ 従来どおり warning
- `ScanAborted`: **走査を完遂できなかった**（ページ予算 `max_pages` / リネーム予算 `max_renames` /
  `git` コマンド自体の失敗——`git log` と `git show --name-status` の両方）→ **error**。これは環境の都合ではなく検査が回っていないことなので、
  warning に落とすと同じ fail-open が一段外側で再現する

番兵は**素の文字列ではなく型**にする（`ScanAborted`）。str にすると、呼び出し側が `is None` だけ
見て番兵を SHA として `merge-base --is-ancestor` に渡し、偽の STALE を出す事故が起きうる。
`reason` に原因を載せるのは、3 経路が同じ文言になると無関係な定数をいじらせてしまうため。
`max_pages` は**走査全体**のページ数で、リネームを辿っても取り直さない。パス単位にすると
実際の上限が `max_renames × max_pages` に膨らんで宣言と乖離し、病的な履歴では `adr` ジョブの
timeout が先に来て「打ち切りを error にする」意図が届かない。`max_renames=N` のとき実際に
辿れるリネームは **N-1 段**（N 段目を見つけた時点で打ち切る）。

**バイト列で比較する（改行コードも不正バイトも潰さない）。** ブロブ取得を `text=True` で行うと
universal newlines が `\r\n` を `\n` に潰し、**CRLF 変換とピン更新が同居したコミットが
「ピン行だけの差分」に見えて免除される**（`run:` ブロックの改行コードは shell の挙動を変えうる）。
同様に不正 UTF-8 を `errors="replace"` で復号すると、**異なるバイト列が同じ U+FFFD に潰れて
「行が一致」に見える**。したがって取得と行比較はバイト列で行い、復号は正規表現に当てる直前だけ・
往復可能な `surrogateescape` で行う（`splitlines()` は `\r` でも切ってしまうので `b"\n"` で分割する）。

#### 理由

**ピンの hex が上がっても、下流 knowledge の本文が語る内容は変わらないから。**
`ci-pipeline.md` が書いているのはジョブ構成と分割の設計意図であって、各 action の版ではない。
下流が読み直す理由が無い変更で追従を強制すると、追従は中身を見ない儀式に落ちる
（[#604](https://github.com/taito-station/paddock/issues/604) 要件 (e) が測ろうとしている劣化そのもの）。

**owner/repo の同一性を条件に入れるのは、action の差し替えはジョブの意味が変わるから。**
`dtolnay/rust-toolchain` を別の toolchain action へ替えるのは設計変更で、`ci-pipeline.md` の
記述が古くなりうる。hex とバージョン注記だけを許す形にすれば、ピン更新は通り、差し替えは止まる。

**タグへの緩和（`@<40hex>` → `@v4`）は片側が正規表現に合わないので自動的に内容変更になる。**
これは意図した挙動で、サプライチェーン対策の後退は下流に伝えるべき信号。

**メジャー更新（`# v4` → `# v7.0.0`）も免除する。** action の入出力やランタイムが変わる更新でも、
本文書が語るのは「どのジョブがあり、なぜ分けたか」なので記述は古くならない（実際 `884f982` は
setup-node の v4 → v7 で、`ci-pipeline.md` のジョブ構成の記述に一切影響しない）。メジャーだけを
内容変更に倒すと、いちばん通したい実例が例外に乗らなくなる。action の挙動変化そのものは
`ci` / `web` などの実ジョブが落ちることで検知される——文書の stale 検査の役割ではない。

**例外 1d は「機械が吸収する」側の例外**（例外 1 / 1b と同列）で、人が bump する例外 1c とは
性質が違う。ここに置くことで、dependabot の素の PR が人手ゼロで通る。

#### 却下した代替案

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
- **hex のみの変更に限定する（末尾注記を許さない）**: dependabot が注記も書き換えることがあり
  （`884f982`）、その形の PR が例外に乗らず問題が解決しない。
- **例外をパスで絞らず全 `sources` に適用する**: 実装は行単位・字面ベースなので、Markdown の
  コードフェンスに書いた `uses:` の見本を書き換えただけでその文書の stale 検査が消える。
  README も ADR も「対象はワークフロー」として書いているので、実装もそう絞るのが規約と一致する。

#### 影響

- dependabot の Actions 更新 PR が人手ゼロで `adr` ジョブを通る。ピン更新と説明コメントの改訂が
  同居する PR（#607 の形）は従来どおり stale になる——これは意図どおりで、
  `scripts/test-check-doc-classes.py` に対照ケースとして固定した。
- 例外が広すぎないことの担保はテストに依存する。ピン行以外の差分が 1 行でも混ざれば
  内容変更に落ちるため、「ci.yml を触った PR は何でも通る」への退行はテストで検出される。
  免除の境界（パスの限定・`.yaml` の許容・ワークフロー以外の `.yml` の除外・owner/repo の 2 要素・
  版注記の形（版の字面と `#` 前の空白）・hex が実際に動いたこと・インデントの同一・行数の一致・
  CRLF 変換との同居・不正バイトの差し替えとの同居）は**それぞれ対照ケースで固定し、
  実装からガードを外すとそのケースが落ちることを mutation で全件確認してある**。
  **差し替え系の対照ケースはピン更新と同居させる形で書く**
  ——hex を据え置くと条件 5 で弾かれてしまい、狙ったガード（owner/repo の同一性・行数の一致）を
  消してもテストが緑のままになる（実際に 1 巡目の実装がその状態だった）。
- 行末の改行コードが変わったコミットは、行末に残る `\r` で正規表現が外れるため内容変更として
  扱われる（保守的側に倒す）。CRLF 変換だけの場合も、ピン更新と同居した場合も同じ。
- **バイト列比較は例外 1b も締める**。従来は `text=True` の復号を通していたため
  「CRLF 変換＋frontmatter のメタデータのみ」のコミットが免除されていたが、今は本文の
  バイト列が変わるので内容変更になる。保守側に倒れるだけなので許容する。
- 走査窓のページングは例外 1d に限らず**例外 1 / 1b にも効く**（従来はメタデータのみの
  コミットが 40 件続いても検査が消えていた）。副作用として、除外対象が長く続く履歴では
  `git log` の呼び出しが走査全体で最大 `max_pages`（25）回まで増える（＝最大 1000 コミット。
  実際の `ci.yml`＝46 コミットでは 1〜2 ページで終わる）。
- **`git` コマンド自体の失敗も `ScanAborted`（error）に寄せる。** `git log` だけでなく
  `git show --name-status`（走査中に最も多く呼ぶ）も対象。失敗を「このコミットは対象パスを
  触っていない」と同じ扱いにすると、最後の内容変更コミットが黙って飛んで**より古い SHA が
  返り stale 検査が静かに通る**。
- **将来 dependabot の auto-merge を入れるなら、ピン差分の監査を別に持つ必要がある。**
  この例外は `adr` ジョブから「ci.yml が変わった」という自動シグナルを外す。owner/repo が
  同一でも、hex が同一リポジトリの未マージ PR の SHA を指せば任意コードが走る既知の攻撃面がある。
  現状 `.github/dependabot.yml` に auto-merge の配線は無く人のレビューが残るので実害はないが、
  自動マージを入れる時点で「許可 owner/repo リスト」か「SHA の到達性検証」が前提になる。
- **既知の限界（本 ADR で塞がないもの）**: (1) マージコミット自身だけが内容を変える
  evil merge は `git show` が既定でマージの差分を出さないため恒久的に不可視（既存の挙動）。
  (2) CRLF で保存されたワークフローは行末の `\r` で正規表現が外れるため、例外 1d が一切
  効かない（fail-closed 側なので実害はないが「なぜ免除されない」の調査を省くために記録する）。
- 同型の構造は `docs/api/openapi.json` を `sources` に持つ specifications 3 本
  （`prediction-search-api.md` / `rest-api-read.md` / `session-write-api.md`）にもあるが、
  生成物なので「実質変更なし」の差分が起きにくく、実害が観測されていない。本 ADR の対象外。
- `sources` に **`.md` 以外**を入れている文書は 4 本しかない。**ワークフロー / コード**は
  ci-pipeline.md（`.github/workflows/ci.yml`）だけ、**生成物**は上記 API 系 3 本
  （`docs/api/openapi.json`）だけ。今後この種類が増えるときは、その種類ごとに
  「内容変更でない差分」の定義が要るかを検討する。

### ADR 0082: Swagger UI を vendored にしてビルド時の外部ダウンロードをやめる (2026-08-13) — 承認済み

#### ステータス

承認済み（[#614](https://github.com/taito-station/paddock/pull/614) で実装）。
対象 Issue: [#606](https://github.com/taito-station/paddock/issues/606)（論点 B）。
関連: ADR 0026（外部依存の版を固定する判断の先例）。

**採番の注記**: ADR 0081（#612・論点 A）と同じ issue の別 PR で、0081 が未マージのため
`scripts/check-adr-numbers.sh next` はこの時点で 0081 を空きと報告する。衝突を避けるため
0082 を明示的に使う。

#### コンテキスト

`api-server` が依存する `utoipa-swagger-ui` の **build script が Swagger UI の zip を
ビルド時に外部から取得する**。既定のダウンロード元は
`https://github.com/swagger-api/swagger-ui/archive/refs/tags/v5.17.14.zip` で、取得は
`curl -sSL` の起動（`reqwest` feature を有効にしていないため）。**リトライ機構が無い**——curl の
実引数は `-sSL -o <path> <url>`（＋ `CARGO_HTTP_CAINFO` があれば `--cacert`）だけで `--retry` を
渡しておらず、build script 側にも再試行が無いので `build.rs:216` の
`download_file(...).expect("failed to download Swagger UI")` で **1 回失敗＝即 panic** する。

##### 実害（2026-08-12。以下の時刻はすべて UTC）

配分定数と Python・ドキュメントしか触っていない PR（#611）が `docker-build (api)` で
2 回連続失敗した。エラーは 2 回ともネットワーク層だが、**壊れ方が違う**。

```
1 回目: failed to open downloaded Swagger UI: InvalidArchive("Could not find EOCD")
2 回目: failed to download Swagger UI: "curl download file exited with error status: exit status: 56"
```

- **1 回目**は curl が exit 0 で終わり、壊れた本体（HTTP エラーボディや途中で切れた応答）を
  そのまま保存した。`-f` を付けていないので curl は HTTP エラーを失敗として扱わない。
  結果 `ZipArchive::new` が `build.rs:219` で EOCD を見つけられず panic する。
- **2 回目**は curl 自身が非 0（56 = 受信中の失敗）で終了し、`download_file` が Err を返して
  `build.rs:216` で panic した。こちらは `-f` の有無に関係なく落ちる。

3 回目の再実行で通った（＝一過性）。#610 は 14:13Z に実ビルドで通過しており、壊れていたのは
その後の数時間だけだった。

##### 切り分けを誤誘導したのは「同一コミットの再実行」だった

当初は「main はレイヤキャッシュで緑になるので上流障害を検知できない」と分析したが、**これは誤り**
だった。切り分けのため main の最後の成功 run を**再実行**したところ `cargo build` のレイヤが
`#14 CACHED` になり、そこから「main はダウンロードを実行しない」と読んでしまったもの。実測では
**関連パスを触った push は main でも実ビルド**している。

| main のコミット | `docker-build (api)` | ログ |
|---|---|---|
| `5ae6466` | **3m45s** | `#14 1.597 Downloaded utoipa-swagger-ui v9.0.2` / `#14 DONE 177.2s` |
| `ae8e33b` | **4m00s** | 同様に実ビルド |
| `eb9b9ce` の**再実行** | 40s | `#14 CACHED`（＝この観測の出どころ） |

レイヤキャッシュはビルドコンテキストの内容でキー付けされるので、**すでにビルド済みの同一ツリーを
再実行したときだけ** RUN がスキップされる。GHA のキャッシュはスコープが分離されており main が
PR ブランチ発の cache を読むこともない。つまり **main / PR の非対称は存在せず、上流が落ちれば
main の CI も落ちる**。

`--mount=type=cache` の中身が `type=gha` のレイヤキャッシュに載らないこと自体は事実だが、それは
「RUN が実行された場合に crate 取得を省けない」理由であって、main / PR の非対称の理由ではない。

**ただし「実ビルド」は関連パスを触った push に限る。** `docker-build` は自前の変更検出を持ち
（`deployments/` / `src/` / `web/` / `Cargo.toml` / `Cargo.lock` / `rust-toolchain.toml` / `ci.yml`）、
これに触らない push は `run=false` でスキップして数秒で緑になる。**緑の `docker-build` を読むときは
「実ビルドで緑」と「対象外でスキップ」を所要時間で見分ける**（前者は分単位、後者は 10 秒未満）
——この区別を落とすと同じ誤読が再生産される。

**この誤読自体が「ビルド時に外部から取ってくる」構造のコストだった**——一過性の失敗を前に、
再実行の結果を根拠に「PR の変更が原因ではないか」と疑う方向へ 3 回の再実行と追試を費やした。
外部取得が無ければこの切り分けは発生しない。

##### issue 本文の前提を 2 点訂正する

- **`docker-build` は required status check ではない。** ruleset `main` の
  `required_status_checks` は `ci` / `web` / `adr` / `predict-check` / `shellcheck` / `ocr-pdf` の
  6 本で、`docker-build` と `db-guards` は非必須（`docker-build` については `ci.yml` のジョブ
  コメントにも明記がある）。したがってこのジョブの赤は merge をブロックしない——実害は
  「赤いノイズ＋切り分けコスト」。issue の案 (d)「required から外す」は**既に満たされている**。
- **一方でより深刻な経路がある。** required の `ci` ジョブは api-server をビルドするため、
  `Swatinem/rust-cache` が miss すれば **required check が同じダウンロードで落ちる**。build script を
  最初に走らせるのは `cargo clippy --locked --workspace --all-targets` で、その後
  `cargo test --locked --workspace --exclude pdf-ocr --exclude pdf-parser -- --test-threads=1` が続く
  （本 ADR で足す vendored 検査はそれより前・cargo を使わないテキスト検査）。
  根治の動機は issue 本文より強い。

#### 決定

**`utoipa-swagger-ui` の `vendored` feature を有効にし、ビルド時の外部取得をやめる。**

```toml
utoipa-swagger-ui = { version = "9", features = ["actix-web", "vendored"] }
```

build script は `CARGO_FEATURE_VENDORED` を**最優先で**分岐し、`utoipa-swagger-ui-vendored`
crate が持つ埋め込みバイト列（`SWAGGER_UI_VENDORED`）を使う。`file:` / `http(s):` の分岐にも
入らないので、**build script は curl を起動せずネットワークにも出ない**（cargo 自体は crates.io を
https で叩くので `ca-certificates` は引き続き要る）。

**「取得をやめた」のではなく「検証とリトライのある経路へ載せ替えた」のが本質。** 旧経路は
`curl -sSL`（`-f` を付けない）で落としたバイト列を**ハッシュを一切固定せず・検証もせず** unzip して
バイナリへ埋め込んでいた——だから HTTP エラーボディが zip として保存され上記の `InvalidArchive` が
出た（TOFU ですらない。TOFU は初回接触時に固定して以降の変化を検出する仕組みだが、旧経路は
何も固定していない）。新経路は `Cargo.lock` に記録された sha256 で検証され、取得失敗は cargo の
transient retry に乗る。**同じ資産を、検証のある経路で取る**ようになる。

併せて `deployments/api.Dockerfile` の builder ステージから **`curl` を外す**。`ca-certificates` は
**base の `rust:1.97-slim-bookworm` に同梱済み**なので明示指定は冗長だが（`importer.Dockerfile` の
builder は入れずに cargo ビルドが通っている実例がある）、base が絞られたときの保険として残す。

**埋め込み版は従来のダウンロード版と同一**（実測）: `utoipa-swagger-ui-vendored` 0.1.2 は
`res/v5.17.14.zip` を同梱し `src/lib.rs` に "Swagger UI version: `5.17.14`" と明記していて、
既定のダウンロード先 `SWAGGER_UI_DOWNLOAD_URL_DEFAULT` も同じ v5.17.14 タグを指す。したがって
`/docs` が配信する資産は 1 バイトも変わらず、`/api-docs/openapi.json` の挙動も不変。

#### 理由

**ビルドの再現性を、上流の稼働状況から切り離すのがいちばん安い。** ADR 0026 で mupdf の版を
イメージタグで固定したのと同じ判断で、「ビルド時に外部から取ってくる」構造そのものを消す。
feature 1 つの追加で済み、コードは 1 行も変わらない。

**メジャー更新でも `/docs` の役割は変わらないので、埋め込み版に追従の負担は乗らない。**
Swagger UI は OpenAPI 仕様を描画する開発者向けの UI で、版が変わっても paddock 側の
API 定義（`utoipa` 本体が生成）には影響しない。

#### 却下した代替案

- **リトライを入れる**: build script にリトライ機構が無いので、`docker/build-push-action` の
  外側で包むしかない。今回のように**上流が数十分落ちる**ケースには効かない。加えて
  「一過性かどうか」の判断を CI に埋め込むことになり、切り分けコストは下がらない。
- **`cache` feature で 2 回目以降を省く**: ダウンロード自体は消えない（OS のキャッシュ
  ディレクトリに zip を残すだけ）。CI は毎回クリーンなランナーなので初回が必ず走る。
- **`SWAGGER_UI_DOWNLOAD_URL=file://...` で自リポの zip を指す**: ネットワークは消えるが、
  数 MB の zip をリポジトリに抱え、パスを Dockerfile と CI の両方に配線する必要がある。
  vendored crate なら Cargo が同じことを管理してくれる。
- **`docker-build` を required から外す**: 既に非必須。かつ required の `ci` が同じ
  ダウンロードを踏むので問題が残る。
- **Swagger UI を dev feature に隔離して本番バイナリから外す**: **現状は外部露出が無い**ので同梱の害が
  実質ない。既定ビルドで `/docs` が消えて開発手順が変わるコストのほうが大きい（YAGNI）。
  **ただし「露出が無い」は次の 2 つに依存する前提条件つきの結論**なので、崩れたら再検討する:
  (1) compose が api を `127.0.0.1:8080` に束縛している（`0.0.0.0` へ変えた時点で `/docs` も晒れる）、
  (2) `deployments/web.nginx.conf` が `/api/` しか proxy しない。なお `/docs` は `app.rs` の `/api`
  スコープの外にあるため、将来 `/api` に認証を入れても `/docs` は保護されない。
- **失敗時のメッセージだけ改善する**（issue の案 (a)）: 切り分けは楽になるが落ちる事実は残る。
  根治が feature 1 つで済むので、緩和策を選ぶ理由が無い。

#### 影響

- `ci` / `docker-build` の両方でビルド時ダウンロードが消え、**上流の稼働状況に依存しなくなる**
  （実ビルドが走る run は main / PR を問わず落ちていたので、両方が救われる）。
  併せて「一過性の失敗を再実行で切り分ける」作業自体が不要になる——上で述べたとおり、その
  切り分けで再実行のキャッシュヒットを誤読したのが今回の遠回りの原因だった。
- 依存が 1 本増える（`utoipa-swagger-ui-vendored` 0.1.2・依存ゼロ・build script なし・
  ライセンスは親と同じ `MIT OR Apache-2.0`）。**出荷される `paddock-api` のサイズは変わらない**
  ——埋め込まれる dist は旧経路と同一の v5.17.14 なので。増えるのは `target/` 内の build script
  バイナリ（`include_bytes!` の +4.4 MB）で、crates.io からの +4.4 MB は同サイズの GitHub
  ダウンロードを置き換えるため cold build の取得量はほぼ相殺する。
- **CVE が出たときの更新経路が変わる**。(1) vendored 有効時は `SWAGGER_UI_DOWNLOAD_URL` が
  **無警告で完全に無視される**（分岐が `file:` / `http(s):` より先）ので、「修正版の URL を差す」
  緊急回避は使えない。残る手は `SWAGGER_UI_OVERWRITE_FOLDER`（展開後の個別ファイルを上書きする
  ので vendored でも効く）か feature を一時的に外すこと。(2) dependabot が届くのは
  **`0.1.x` の範囲内だけ**——`utoipa-swagger-ui` 側の build-dependency 要件が `version = "0.1"`
  なので、上流が `0.2.0` で Swagger UI を上げても親が要件を上げるまで伝わらない。
  (3) **検知手段はゼロではない**: `.github/workflows/audit.yml` の `cargo audit`（週次・
  `rustsec/audit-check` が `Cargo.lock` を照合）は新しい crate も射程に入るので、RustSec に
  advisory が立てば拾える。**ただし Swagger UI 本体（JS）の CVE は RustSec に載らないので届かない**
  ——これは vendored 化の前後で変わらない（旧経路も版を固定してダウンロードしていた）。
- **`vendored` が落ちる退行を機械で固定する**（`scripts/check-vendored-swagger.sh`。required の
  `ci` ジョブと pre-push で走る）。feature が外れると build script は**無警告で**ダウンロード分岐へ
  戻り、GitHub ランナーには curl があるので **required の `ci` は黙って外部取得を再開**し、落ちるのは
  非必須の `docker-build` だけ（原因の分かりにくいエラーで）。Dockerfile のコメントは人手の規律に
  すぎないので、ADR 0073 の「人手の規律に委ねない」に合わせて検査を置く。
  判定は 2 段で、**主たる根拠は `Cargo.lock`**——optional な依存は feature で活性化されない限り
  ロックに載らないので、`utoipa-swagger-ui-vendored` の在否がそのまま feature の効き方を表し、
  書式にも依存しない（`--locked` でロックの鮮度も担保される）。宣言側も併せて見るのは、
  「宣言を消したがロックを再生成していない」状態を見逃さないため。**宣言の照合は単一行に
  限定しない**——`features` を複数行に整形するのは正当なので、単一行の grep にすると整形だけで
  検査が落ちる（実際に偽陽性を作ってしまい、宣言の開始行から最初の `}` までを切り出す形に直した）。
- `api.Dockerfile` の builder から `curl` が消える。将来ビルド時に curl が必要な依存を足すときは
  戻す必要がある（コメントに理由を残した）。
- **`docker-build` が非必須である事実は変えない。** このジョブは「builder ステージまで通るか」の
  スモークテストで、required にするかは別の判断（本 ADR の範囲外）。

### ADR 0084: evil merge は stale 検査から見えている（ADR 0081 の「既知の限界 (1)」の訂正） (2026-08-14) — 承認済み

#### ステータス

承認済み。[#615](https://github.com/taito-station/paddock/issues/615) (a)。
**ADR 0081 の「既知の限界 (1)」を訂正する**（決定そのものは覆さない。0081 の例外 1d は有効）。

#### コンテキスト

ADR 0081（ピン更新のみを stale 例外にする）のセルフレビューで、

> マージコミット自身だけが内容を変える evil merge は `git show` が既定でマージの差分を
> 出さないため恒久的に不可視（既存の挙動）

という指摘が出て、ADR 0081 の「既知の限界 (1)」として記録された。`scripts/check-doc-classes.py`
の `scan_last_content_change` にも同趣旨のコメントが置かれていた。#615 (a) はこれを消化する。

**指摘どおりなら fail-open**（`sources` に挙げたファイルが evil merge で書き換えられても
stale 検査が永久に気づかない）なので、まず実際に起こりうるか・本当に不可視かを実測した。

#### 実測

##### 1. 本リポジトリの履歴（全 369 マージ × `sources` 110 パス）

`git log -- <path>` が列挙したマージ × `sources` パスは **7 組**。そのすべてを `path_status` が
検出した（`MM` / `MA` / `AA`）。**不可視は 0 組**。

evil merge は現に起きている——例えば `8ec61a18`（#613 の main 取り込み）は、コンフリクトを
手で解決して**どちらの親にも無い内容**を作っており、`docs/qa/QA-sources-coverage-checks-596.md`
が `MA` で検出されて `last_content_change` はそのマージを返した。PR ブランチが main を
取り込んでコンフリクトを解消する運用がある以上、evil merge は日常的に発生する。

##### 2. 合成 fixture（使い捨て git リポジトリ）

| # | 構成 | `path_status` | `last_content_change` |
|---|---|---|---|
| A | 真の evil merge（両親と異なる内容をマージで作る） | `MM` | **マージ自身** |
| B | 片親の内容をそのまま採るマージ | `None` | 祖先コミット（正しい） |
| C | マージ内での純粋なリネーム（内容差分ゼロ） | `RR` | マージ自身（＝偽 STALE） |
| D | octopus merge（3 親）での evil merge | `MMM` | **マージ自身** |

##### 3. 機構

`git show` はマージに対して**既定で combined diff（`--cc`）**を出し、`--cc` は
**「全ての親と異なるパス」だけ**を列挙する。これは evil merge の定義そのもの。
逆に片親と同じ内容になったマージ（B）は `--cc` が列挙しないが、**`git log` の既定の単純化も
そのマージを列挙しない**（TREESAME な親を辿る）ので、走査がそこへ来ることが無い。

##### 4. `status is None` の分岐は何なのか（マージとは無関係）

上の議論は「`git log` が返す集合」と「`path_status` が status を返す集合」が一致することに
懸かっているので、**`status is None` の分岐に到達するかを計装して数えた**。

| 対象 | `status is None` の到達回数 |
|---|---|
| 実リポジトリ全体（`sources` 110 パス・全履歴） | **0 回** |
| 回帰テスト 183 ケース（**この分岐を pin するテストを足す前**） | **1 回**（非正規形パスの fixture） |
| 回帰テスト 184 ケース（pin テスト追加後） | **2 回**（上記 ＋ 新テストが意図的に踏む 1 回） |

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

#### 決定

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

#### 理由

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

#### 却下した代替案

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

#### 影響

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
  `test_rename_inside_merge_...` が落ちたなら「既知の偽陽性がたまたま消えた」ように見えるが、
  実際には第 1 親比較へ変わって**fail-open 側に穴が開いている**（片側だけの変更では STALE が
  増えるのではなく消える。実測）——**反転させてよい合図ではない**。
- 関連: ADR 0081（例外 1d と `ScanAborted` の error 昇格）/ ADR 0073（機械検査の導入）/
  ADR 0083（`sources` の網羅性検査）/ #612 / #615。

#### 再現方法

```sh
# 契約が守られていること
python3 scripts/test-check-doc-classes.py

# 契約テストが本当に効くこと（変異テスト）:
# path_status の先頭に「マージなら (None, None) を返す」を差し込むと
# test_evil_merge_is_detected_as_content_change が落ちる

# 本リポジトリでの実測（マージ × sources パスのうち不可視が 0 件であること）
# … git log -- <path> が列挙したマージに path_status を当てて数える
```
