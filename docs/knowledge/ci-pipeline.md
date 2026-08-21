---
status: Confirmed
kind: knowledge
doc_class: [D21, D19, D17]
tags: [D21, D19, D17]
sources:
  - docs/original-docs/0026-ocr-pdf-ci-mupdf-pin.md
  - docs/original-docs/0073-adr-into-original-docs-and-doc-classes.md
  - docs/original-docs/0081-pin-only-diff-is-not-content-change.md
  - docs/original-docs/0082-swagger-ui-vendored.md
  - docs/original-docs/0084-evil-merge-is-visible-to-stale-check.md
  - docs/original-docs/616-docs-serving-checks.md
  - docs/original-docs/636-fullwidth-after-var.md
  - docs/qa/QA-evil-merge-615.md
  - docs/qa/QA-fullwidth-after-var-636.md
  - .github/workflows/ci.yml
distilled_from_sha: "b888362"
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

- ADR: [0026 OCR/PDF 統合テストを CI で実走（mupdf 版固定）](../original-docs/0026-ocr-pdf-ci-mupdf-pin.md) /
  [0073 ADR 統合と文書クラス・機械検査](../original-docs/0073-adr-into-original-docs-and-doc-classes.md) /
  [0081 ピン更新だけの差分は内容変更と見なさない](../original-docs/0081-pin-only-diff-is-not-content-change.md) /
  [0082 Swagger UI を vendored にしてビルド時ダウンロードを消す](../original-docs/0082-swagger-ui-vendored.md)
- 必須チェックの ruleset は #461（ジョブ ID `adr` は必須チェック名なので改名しない）
- pre-push は CI 相当の高速チェックを手元で再現する（`scripts/git-hooks/pre-push`。配線は
  `scripts/install-git-hooks.sh` で clone ごとに一度）
