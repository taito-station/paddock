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
distilled_from_sha: "fd34f64"
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

### ビルド時に外部から資産を取ってこない（Swagger UI は vendored・ADR 0082）

`api-server` が依存する `utoipa-swagger-ui` の **build script は Swagger UI の zip をビルド時に
外部から取得していた**（`curl -sSL` の起動。**リトライ機構は無く 1 回失敗＝即 panic**）。上流が
不調だと `docker-build (api)` が落ち、2026-08-12 には配分定数と Python しか触っていない PR（#611）が
2 回連続で失敗した（`InvalidArchive("Could not find EOCD")` と `curl exit status 56`——`-f` を付けない
curl が HTTP エラーボディを zip として保存するため症状が 2 通りに出る）。

**いちばん厄介なのは main が緑のままになること。** main の run では `cargo build` のレイヤが
`CACHED` でダウンロードを一度も実行しない。`--mount=type=cache` の中身は `type=gha` の
レイヤキャッシュに載らないので、Rust ソースを触る PR 側だけが実ダウンロードを踏む。結果
「main は緑なのに PR だけ落ちる」（PR の変更が疑われるが無関係）と「上流が壊れていても main の
CI は緑を出し続ける」の 2 つの誤誘導が起きる。

**決定**: `vendored` feature を有効にして外部取得をやめる。build script は
`CARGO_FEATURE_VENDORED` を最優先で分岐し、`utoipa-swagger-ui-vendored` の埋め込みバイト列を
使うので curl も CA 証明書もネットワークも要らない。併せて `api.Dockerfile` の builder から
`curl` を外す（`ca-certificates` は cargo の crates.io 取得に要るので残す）。`/docs` と
`/api-docs/openapi.json` の挙動は変わらない。

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
  `cargo test --locked --workspace` で api-server をビルドするため同じダウンロードを踏む——
  `Swatinem/rust-cache` が miss すれば required check が上流障害で落ちる。**根治の動機はこちら。**
- **Swagger UI を dev feature に隔離して本番バイナリから外す**: `/docs` は compose の
  `127.0.0.1:8080` 限定公開で `deployments/web.nginx.conf` は `/api/` しか proxy しない＝外部露出が
  無い。同梱の害が実質ないのに既定ビルドで `/docs` が消えるコストのほうが大きい（YAGNI）。
- **失敗時のメッセージだけ改善する**: 切り分けは楽になるが落ちる事実は残る。根治が feature 1 つで
  済むので緩和策を選ぶ理由が無い。

**影響**: `ci` / `docker-build` の両方でビルド時ダウンロードが消え、上流の稼働状況に依存しなくなる
（「main はキャッシュで緑・PR だけ落ちる」の非対称も消える）。依存が 1 本増え、埋め込み zip のぶん
取得サイズとビルド成果物が増えるが実行時の挙動は不変。**Swagger UI の版は vendored crate が決める**
ので既定のダウンロード先（v5.17.14）と一致しない可能性があり、`/docs` の描画はブラウザで目視確認する
（以降の版更新は dependabot の cargo エコシステム経由）。builder から `curl` が消えたので、将来
ビルド時に curl が必要な依存を足すときは戻す（Dockerfile のコメントに理由を残した）。
`docker-build` が非必須である事実は変えない——required にするかは別の判断。

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
