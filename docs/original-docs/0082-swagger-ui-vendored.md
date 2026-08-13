# 0082. Swagger UI を vendored にしてビルド時の外部ダウンロードをやめる

## ステータス

承認済み（本 PR で実装）。対象 Issue: [#606](https://github.com/taito-station/paddock/issues/606)（論点 B）。
関連: [ADR 0026](0026-ocr-pdf-ci-mupdf-pin.md)（外部依存の版を固定する判断の先例）。

**採番の注記**: ADR 0081（#612・論点 A）と同じ issue の別 PR で、0081 が未マージのため
`scripts/check-adr-numbers.sh next` はこの時点で 0081 を空きと報告する。衝突を避けるため
0082 を明示的に使う。

## コンテキスト

`api-server` が依存する `utoipa-swagger-ui` の **build script が Swagger UI の zip を
ビルド時に外部から取得する**。既定のダウンロード元は
`https://github.com/swagger-api/swagger-ui/archive/refs/tags/v5.17.14.zip` で、取得は
`curl -sSL` の起動（`reqwest` feature を有効にしていないため）。**build.rs 383 行に
リトライ機構は無く**、`download_file(...).expect("failed to download Swagger UI")` で
1 回失敗＝即 panic する。

### 実害（2026-08-12）

配分定数と Python・ドキュメントしか触っていない PR（#611）が `docker-build (api)` で
2 回連続失敗した。エラーは 2 回ともネットワーク層で、`-f` を付けない curl が HTTP エラー
ボディや途中切れを zip として保存するため症状が 2 通りに出る。

```
1 回目: failed to open downloaded Swagger UI: InvalidArchive("Could not find EOCD")
2 回目: failed to download Swagger UI: "curl download file exited with error status: exit status: 56"
```

3 回目の再実行で通った（＝一過性）。#610 は同日 13:50 頃に実ビルドで通過しており、
壊れていたのは 14:13〜18:00 の間だけだった。

### いちばん厄介な点: main は緑のままになる

切り分けのため main の最後の成功 run で同じジョブを再実行したところ success だったが、
ログでは `cargo build` のレイヤが `#14 CACHED` で、**ダウンロードを一度も実行していない**。
PR 側は Rust ソースを触るのでキャッシュが崩れ、実際にダウンロードが走る。`--mount=type=cache`
の中身は `type=gha` のレイヤキャッシュに載らないため、この非対称は構造的に生じる。結果:

- **「main は緑なのに PR だけ落ちる」→ PR の変更が疑われる**（実際は無関係）
- **上流が壊れていても main の CI は緑を出し続ける**（検知できない）

今回は切り分けに 3 回の再実行と main 側の追試が要った。

### issue 本文の前提を 2 点訂正する

- **`docker-build` は required status check ではない。** ruleset `main` の
  `required_status_checks` は `ci` / `web` / `adr` / `predict-check` / `shellcheck` / `ocr-pdf` の
  6 本で、`docker-build` と `db-guards` は非必須（`ci.yml` の設計コメントにも明記）。
  したがってこのジョブの赤は merge をブロックしない——実害は「赤いノイズ＋切り分けコスト」。
  issue の案 (d)「required から外す」は**既に満たされている**。
- **一方でより深刻な経路がある。** required の `ci` ジョブが `cargo test --locked --workspace` で
  api-server をビルドするため、`Swatinem/rust-cache` が miss すれば **required check が同じ
  ダウンロードで落ちる**。根治の動機は issue 本文より強い。

## 決定

**`utoipa-swagger-ui` の `vendored` feature を有効にし、ビルド時の外部取得をやめる。**

```toml
utoipa-swagger-ui = { version = "9", features = ["actix-web", "vendored"] }
```

build script は `CARGO_FEATURE_VENDORED` を**最優先で**分岐し、`utoipa-swagger-ui-vendored`
crate が持つ埋め込みバイト列（`SWAGGER_UI_VENDORED`）を使う。`file:` / `http(s):` の分岐にも
入らないので、**curl も CA 証明書もネットワークも要らない**。

併せて `deployments/api.Dockerfile` の builder ステージから **`curl` を外す**（`ca-certificates`
は cargo が crates.io から依存を取るのに必要なので残す）。理由コメントも実態に合わせる。

`/docs` の Swagger UI と `/api-docs/openapi.json` の挙動は変わらない。

## 理由

**ビルドの再現性を、上流の稼働状況から切り離すのがいちばん安い。** ADR 0026 で mupdf の版を
イメージタグで固定したのと同じ判断で、「ビルド時に外部から取ってくる」構造そのものを消す。
feature 1 つの追加で済み、コードは 1 行も変わらない。

**メジャー更新でも `/docs` の役割は変わらないので、埋め込み版に追従の負担は乗らない。**
Swagger UI は OpenAPI 仕様を描画する開発者向けの UI で、版が変わっても paddock 側の
API 定義（`utoipa` 本体が生成）には影響しない。

## 却下した代替案

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
- **Swagger UI を dev feature に隔離して本番バイナリから外す**: `/docs` は compose の
  `127.0.0.1:8080` 限定公開で、`deployments/web.nginx.conf` は `/api/` しか proxy しないため
  **外部露出は無い**。同梱の害が実質ないのに、既定ビルドで `/docs` が消えて開発手順が変わる
  コストのほうが大きい（YAGNI）。
- **失敗時のメッセージだけ改善する**（issue の案 (a)）: 切り分けは楽になるが落ちる事実は残る。
  根治が feature 1 つで済むので、緩和策を選ぶ理由が無い。

## 影響

- `ci` / `docker-build` の両方でビルド時ダウンロードが消え、**上流の稼働状況に依存しなくなる**。
  「main はキャッシュで緑・PR だけ落ちる」という非対称も、ダウンロード自体が無くなるので消える。
- 依存が 1 本増える（`utoipa-swagger-ui-vendored`）。埋め込み zip のぶんだけ crate の取得サイズと
  ビルド成果物が増えるが、実行時の挙動は変わらない。
- **Swagger UI の版は vendored crate が決める**。既定のダウンロード先（v5.17.14）と一致しない
  可能性があるので、`/docs` の描画はブラウザで目視確認する。以降の版更新は dependabot の
  cargo エコシステム経由で入る。
- `api.Dockerfile` の builder から `curl` が消える。将来ビルド時に curl が必要な依存を足すときは
  戻す必要がある（コメントに理由を残した）。
- **`docker-build` が非必須である事実は変えない。** このジョブは「builder ステージまで通るか」の
  スモークテストで、required にするかは別の判断（本 ADR の範囲外）。
