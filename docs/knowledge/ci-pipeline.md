---
status: Confirmed
kind: knowledge
doc_class: [D21, D19, D17]
tags: [D21, D19, D17]
sources:
  - docs/original-docs/0026-ocr-pdf-ci-mupdf-pin.md
  - docs/original-docs/0073-adr-into-original-docs-and-doc-classes.md
  - docs/original-docs/0082-swagger-ui-vendored.md
  - .github/workflows/ci.yml
distilled_from_sha: "e374ee2"
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

### build script が lock/checksum 外の資産を取ってこない（Swagger UI は vendored・ADR 0082）

`api-server` が依存する `utoipa-swagger-ui` の **build script は Swagger UI の zip をビルド時に
外部から取得していた**（`curl -sSL -o <path> <url>` の起動。**`--retry` も build script 側の再試行も
無く `build.rs:216` で 1 回失敗＝即 panic**）。上流が不調だと `docker-build (api)` が落ち、
2026-08-12〜13（UTC）には配分定数と Python しか触っていない PR（#611）が 2 回連続で失敗した。
**壊れ方は 2 通り**で、1 回目は curl が exit 0 のまま壊れた本体を保存し（`-f` 無しなので HTTP
エラーを失敗と扱わない）`ZipArchive::new` が EOCD を見つけられず panic、2 回目は curl 自身が
非 0（56）で終了して `download_file` が Err を返した（こちらは `-f` の有無に関係なく落ちる）。

**「main はキャッシュで緑になるから上流障害を検知できない」という当初の分析は誤りだった。**
実測では main への push は毎回**実ビルド**している（`5ae6466` は `docker-build (api)` 3m45s で
ログに `Downloaded utoipa-swagger-ui v9.0.2` / `#14 DONE 177.2s`、`ae8e33b` は 4m00s）。
`#14 CACHED` が出たのは**すでにビルド済みの同一コミットを再実行したとき**だけ（`eb9b9ce` の
再実行・40s）。レイヤキャッシュはビルドコンテキストの内容でキー付けされ、GHA のキャッシュは
スコープが分離されていて main が PR ブランチ発の cache を読むこともないので、**main / PR の
非対称は存在せず上流が落ちれば main も落ちる**。`--mount=type=cache` の中身が `type=gha` に
載らないのは「RUN が走ったときに crate 取得を省けない」理由であって、非対称の理由ではない。
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
TOFU で、だから HTTP エラーボディが zip として保存され上記の `InvalidArchive` が出た。新経路は
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
  `cargo test --locked --workspace --exclude pdf-ocr --exclude pdf-parser -- --test-threads=1` で
  api-server をビルドするため同じダウンロードを踏む——
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
  （main も PR も等しく実ビルドするので、以前は上流が落ちればどちらも落ちていた）。併せて
  「一過性の失敗を再実行で切り分ける」作業自体が不要になる。
- 依存が 1 本増える（`utoipa-swagger-ui-vendored` 0.1.2・依存ゼロ・build script なし・ライセンスは
  親と同じ `MIT OR Apache-2.0`）。**出荷されるバイナリのサイズは変わらない**（埋め込む dist が旧経路と
  同一なので）。増えるのは `target/` 内の build script バイナリ（+4.4 MB）で、crates.io からの +4.4 MB は
  同サイズの GitHub ダウンロードを置き換えるため cold build の取得量はほぼ相殺する。
- **CVE が出たときの更新経路が変わる**。vendored 有効時は `SWAGGER_UI_DOWNLOAD_URL` が**無警告で
  完全に無視される**ので「修正版の URL を差す」緊急回避は使えない（残る手は
  `SWAGGER_UI_OVERWRITE_FOLDER` か feature を一時的に外すこと）。dependabot が届くのも
  **`0.1.x` の範囲内だけ**——`utoipa-swagger-ui` の build-dependency 要件が `version = "0.1"` なので、
  上流が `0.2.0` で Swagger UI を上げても親が要件を上げるまで伝わらない。
- **`vendored` が落ちる退行は機械で固定する**（`scripts/check-vendored-swagger.sh`。required の `ci`
  ジョブと pre-push）。feature が外れると build script は無警告でダウンロード分岐へ戻り、GitHub
  ランナーには curl があるので **required の `ci` は黙って外部取得を再開**する（落ちるのは非必須の
  `docker-build` だけ・原因の分かりにくいエラーで）。Dockerfile のコメントは人手の規律にすぎない。
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
  [0082 Swagger UI を vendored にしてビルド時ダウンロードを消す](../original-docs/0082-swagger-ui-vendored.md)
- 必須チェックの ruleset は #461（ジョブ ID `adr` は必須チェック名なので改名しない）
- pre-push は CI 相当の高速チェックを手元で再現する（`scripts/git-hooks/pre-push`。配線は
  `scripts/install-git-hooks.sh` で clone ごとに一度）
