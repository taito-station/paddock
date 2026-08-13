# 0082. Swagger UI を vendored にしてビルド時の外部ダウンロードをやめる

## ステータス

承認済み（[#614](https://github.com/taito-station/paddock/pull/614) で実装）。
対象 Issue: [#606](https://github.com/taito-station/paddock/issues/606)（論点 B）。
関連: [ADR 0026](0026-ocr-pdf-ci-mupdf-pin.md)（外部依存の版を固定する判断の先例）。

**採番の注記**: ADR 0081（#612・論点 A）と同じ issue の別 PR で、0081 が未マージのため
`scripts/check-adr-numbers.sh next` はこの時点で 0081 を空きと報告する。衝突を避けるため
0082 を明示的に使う。

## コンテキスト

`api-server` が依存する `utoipa-swagger-ui` の **build script が Swagger UI の zip を
ビルド時に外部から取得する**。既定のダウンロード元は
`https://github.com/swagger-api/swagger-ui/archive/refs/tags/v5.17.14.zip` で、取得は
`curl -sSL` の起動（`reqwest` feature を有効にしていないため）。**リトライ機構が無い**——curl の
実引数は `-sSL -o <path> <url>`（＋ `CARGO_HTTP_CAINFO` があれば `--cacert`）だけで `--retry` を
渡しておらず、build script 側にも再試行が無いので `build.rs:216` の
`download_file(...).expect("failed to download Swagger UI")` で **1 回失敗＝即 panic** する。

### 実害（2026-08-12〜13。以下の時刻は UTC）

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

### 切り分けを誤誘導したのは「同一コミットの再実行」だった

当初は「main はレイヤキャッシュで緑になるので上流障害を検知できない」と分析したが、**これは誤り**
だった。切り分けのため main の最後の成功 run を**再実行**したところ `cargo build` のレイヤが
`#14 CACHED` になり、そこから「main はダウンロードを実行しない」と読んでしまったもの。実測では
main への push は毎回**実ビルド**している。

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

**この誤読自体が「ビルド時に外部から取ってくる」構造のコストだった**——一過性の失敗を前に、
再実行の結果を根拠に「PR の変更が原因ではないか」と疑う方向へ 3 回の再実行と追試を費やした。
外部取得が無ければこの切り分けは発生しない。

### issue 本文の前提を 2 点訂正する

- **`docker-build` は required status check ではない。** ruleset `main` の
  `required_status_checks` は `ci` / `web` / `adr` / `predict-check` / `shellcheck` / `ocr-pdf` の
  6 本で、`docker-build` と `db-guards` は非必須（`docker-build` については `ci.yml` のジョブ
  コメントにも明記がある）。したがってこのジョブの赤は merge をブロックしない——実害は
  「赤いノイズ＋切り分けコスト」。issue の案 (d)「required から外す」は**既に満たされている**。
- **一方でより深刻な経路がある。** required の `ci` ジョブは api-server をビルドするため、
  `Swatinem/rust-cache` が miss すれば **required check が同じダウンロードで落ちる**。実際に
  build script を先に走らせるのは `cargo clippy --locked --workspace --all-targets`（テストより前）で、
  その後 `cargo test --locked --workspace --exclude pdf-ocr --exclude pdf-parser -- --test-threads=1`
  が続く。根治の動機は issue 本文より強い。

## 決定

**`utoipa-swagger-ui` の `vendored` feature を有効にし、ビルド時の外部取得をやめる。**

```toml
utoipa-swagger-ui = { version = "9", features = ["actix-web", "vendored"] }
```

build script は `CARGO_FEATURE_VENDORED` を**最優先で**分岐し、`utoipa-swagger-ui-vendored`
crate が持つ埋め込みバイト列（`SWAGGER_UI_VENDORED`）を使う。`file:` / `http(s):` の分岐にも
入らないので、**build script は curl を起動せずネットワークにも出ない**（cargo 自体は crates.io を
https で叩くので `ca-certificates` は引き続き要る）。

**「取得をやめた」のではなく「検証とリトライのある経路へ載せ替えた」のが本質。** 旧経路は
`curl -sSL`（`-f` を付けない）で落としたバイト列を**ハッシュ検証なしに** unzip してバイナリへ
埋め込む TOFU（trust on first use）だった——だから HTTP エラーボディが zip として保存され
`Could not find EOCD` が出た。新経路は `Cargo.lock` に記録された sha256 で検証され、取得失敗は
cargo の transient retry に乗る。**同じ資産を、検証のある経路で取る**ようになる。

併せて `deployments/api.Dockerfile` の builder ステージから **`curl` を外す**。`ca-certificates` は
**base の `rust:1.97-slim-bookworm` に同梱済み**なので明示指定は冗長だが（`importer.Dockerfile` の
builder は入れずに cargo ビルドが通っている実例がある）、base が絞られたときの保険として残す。

**埋め込み版は従来のダウンロード版と同一**（実測）: `utoipa-swagger-ui-vendored` 0.1.2 は
`res/v5.17.14.zip` を同梱し `src/lib.rs` に "Swagger UI version: `5.17.14`" と明記していて、
既定のダウンロード先 `SWAGGER_UI_DOWNLOAD_URL_DEFAULT` も同じ v5.17.14 タグを指す。したがって
`/docs` が配信する資産は 1 バイトも変わらず、`/api-docs/openapi.json` の挙動も不変。

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
- **Swagger UI を dev feature に隔離して本番バイナリから外す**: **現状は外部露出が無い**ので同梱の害が
  実質ない。既定ビルドで `/docs` が消えて開発手順が変わるコストのほうが大きい（YAGNI）。
  **ただし「露出が無い」は次の 2 つに依存する前提条件つきの結論**なので、崩れたら再検討する:
  (1) compose が api を `127.0.0.1:8080` に束縛している（`0.0.0.0` へ変えた時点で `/docs` も晒れる）、
  (2) `deployments/web.nginx.conf` が `/api/` しか proxy しない。なお `/docs` は `app.rs` の `/api`
  スコープの外にあるため、将来 `/api` に認証を入れても `/docs` は保護されない。
- **失敗時のメッセージだけ改善する**（issue の案 (a)）: 切り分けは楽になるが落ちる事実は残る。
  根治が feature 1 つで済むので、緩和策を選ぶ理由が無い。

## 影響

- `ci` / `docker-build` の両方でビルド時ダウンロードが消え、**上流の稼働状況に依存しなくなる**。
  「main はキャッシュで緑・PR だけ落ちる」という非対称も、ダウンロード自体が無くなるので消える。
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
- **`vendored` が落ちる退行を機械で固定する**（`scripts/check-vendored-swagger.sh`。required の
  `ci` ジョブと pre-push で走る）。feature が外れると build script は**無警告で**ダウンロード分岐へ
  戻り、GitHub ランナーには curl があるので **required の `ci` は黙って外部取得を再開**し、落ちるのは
  非必須の `docker-build` だけ（原因の分かりにくいエラーで）。Dockerfile のコメントは人手の規律に
  すぎないので、ADR 0073 の「人手の規律に委ねない」に合わせて検査を置く。
- `api.Dockerfile` の builder から `curl` が消える。将来ビルド時に curl が必要な依存を足すときは
  戻す必要がある（コメントに理由を残した）。
- **`docker-build` が非必須である事実は変えない。** このジョブは「builder ステージまで通るか」の
  スモークテストで、required にするかは別の判断（本 ADR の範囲外）。
