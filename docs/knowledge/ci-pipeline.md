---
status: Confirmed
kind: knowledge
doc_class: [D21, D19, D17]
tags: [D21, D19, D17]
sources:
  - docs/original-docs/0026-ocr-pdf-ci-mupdf-pin.md
  - docs/original-docs/0073-adr-into-original-docs-and-doc-classes.md
  - docs/original-docs/0081-pin-only-diff-is-not-content-change.md
  - .github/workflows/ci.yml
distilled_from_sha: "0341cc4"
updated: "2026-08-13"
---

# CI パイプラインの構成と設計意図（D21）

`.github/workflows/ci.yml` の**ジョブ分割がなぜこの形なのか**を書く。ジョブ一覧はワークフローを見れば
分かるが、「なぜ分けたか」「なぜこの版に固定したか」はコードに書けないのでここが正になる。

D21（CI/CD・ビルド・リリース・供給網管理）の充足ギャップを埋める文書
（[doc-classes.md](doc-classes.md) 参照）。

## ジョブ構成（8 ジョブ + マトリクス 3）

| ジョブ | 実行環境 | 内容 |
|---|---|---|
| `ci` | ubuntu-latest ＋ postgres サービス | toolchain 一致 assert / fmt / clippy / `cargo test`（**直列**・OCR・PDF crate を除く） |
| `web` | ubuntu-latest | typecheck / eslint / vitest / **生成 API 型のドリフト検証** / vite build |
| `adr` | ubuntu-latest | ADR 番号重複と文書クラス・sources の検査（**回帰テスト → 本番検査**の順） |
| `predict-check` | ubuntu-latest | stdlib のみの Python テスト（自走式 + ハーネス忠実性） |
| `shellcheck` | ubuntu-latest | `shellcheck --severity=warning` |
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
＝ページ予算 `max_pages` / リネーム予算 `max_renames` / `git log` 自体の失敗 → **error**）。
未完遂を `None` に混ぜると同じ fail-open が一段外側で再現するので分ける。番兵は素の文字列ではなく
型にする（str だと呼び出し側が SHA として `merge-base` へ渡し偽 STALE を出しうる）。予算は
**パス単位**なので全体の上限は `max_renames × max_pages` ページ、`max_renames=N` で実際に辿れる
リネームは N-1 段。この修正は例外 1 / 1b にも効く。

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

**影響**: ピン更新 PR は人手ゼロで `adr` を通る。ピン更新と説明コメントの改訂が同居する PR
（#607 の形）は従来どおり stale になり、これは意図どおりで `scripts/test-check-doc-classes.py` に
対照ケースとして固定した（新規 31 ケース）。例外が広すぎないことの担保はこのテストに依存する
——ピン行以外の差分が 1 行でも混ざれば内容変更に落ちるので、「`ci.yml` を触った PR は何でも通る」
への退行はテストで検出される。免除の境界（パスの限定・`.yaml` の許容・ワークフロー以外の `.yml` の
除外・owner/repo の 2 要素・版注記の形・hex が実際に動いたこと・インデントの同一・行数の一致・
CRLF 変換との同居・不正バイトの差し替えとの同居）も**それぞれ対照ケースで固定し、実装からガードを
外すとそのケースが落ちることを mutation で全件確認してある**（15 ガード）。**「ガードを消しても
テストが緑」を避けるため、差し替え系の対照ケースはピン更新と同居させる形で書く**（hex を据え置くと
別の条件で弾かれて狙ったガードを固定できない）。走査窓のページングは例外 1 / 1b にも効く代わりに、除外対象が長く続く
履歴では 1 パスあたり `git log` の呼び出しが最大 `max_pages`（25）回まで増える。リネームを辿るときは
パスが変わるのでページ予算と窓の位置を取り直すため、**全体の上限は `max_renames × max_pages`
ページ**になる（現実の `ci.yml`＝46 コミットでは 1〜2 ページで終わる）。同型の構造は `docs/api/openapi.json` を `sources` に
持つ specifications 3 本にもあるが、生成物なので「実質変更なし」の差分が起きにくく実害が観測されて
いないため対象外。`sources` に `.md` 以外を入れている文書は 4 本だけ（**ワークフロー / コード**は
本文書の `ci.yml` のみ、**生成物**は API 系 3 本の `openapi.json` のみ）で、今後この種類が増えるときは
種類ごとに「内容変更でない差分」の定義が要るかを検討する。

**将来 dependabot の auto-merge を入れるなら、ピン差分の監査を別に持つ必要がある。** この例外は
`adr` ジョブから「`ci.yml` が変わった」という自動シグナルを外す。owner/repo が同一でも、hex が
同一リポジトリの未マージ PR の SHA を指せば任意コードが走る既知の攻撃面がある。現状
`.github/dependabot.yml` に auto-merge の配線は無く人のレビューが残るので実害はないが、
自動マージを入れる時点で「許可 owner/repo リスト」か「SHA の到達性検証」が前提になる。

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
  [0081 ピン更新だけの差分は内容変更と見なさない](../original-docs/0081-pin-only-diff-is-not-content-change.md)
- 必須チェックの ruleset は #461（ジョブ ID `adr` は必須チェック名なので改名しない）
- pre-push は CI 相当の高速チェックを手元で再現する（`scripts/git-hooks/pre-push`。配線は
  `scripts/install-git-hooks.sh` で clone ごとに一度）
