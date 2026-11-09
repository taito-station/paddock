# QA: cargo の依存の脆弱性（#825・2026-11-08〜09）

#820 で直した週次の Security Audit が検出した、`Cargo.lock` の脆弱性 20 件を直すにあたり、着手時に確定した点。
回答は会話でユーザーが選んだもの（AskUserQuestion）。
蒸留先: `knowledge/rest-api-read.md`（決定ログ #825）。

## 着手時に確かめた事実

- 対象の crate だけを `cargo update -p` で上げると、20 件のうち 19 件が Cargo.lock の更新だけで直る。zip 9 系も utoipa-swagger-ui 10.0.2 が取り込むので、Cargo.toml の変更は要らない。
- 残るのは h2 0.3.27 の RUSTSEC-2026-0258（Low）。
  - 修正版は 0.4.16 以降だけ。
  - actix-http の最新（3.20.5）も h2 0.3 に依存する。
  - h2 は actix-web の既定の feature `http2` からだけ入る（utoipa-swagger-ui が有効にするのは `macros` だけ）。
- api-server は Compress ミドルウェア・cookie・TLS を使っていない（`src/` を grep して確認）。
- 平文の `HttpServer::bind` は `HttpService::tcp()` を通って HTTP/1.1 に固定される（actix-http 3.20.5 の `service.rs`。h2c は `bind_auto_h2c`、HTTP/2 は TLS の ALPN でしか受けない）。api-server はもともと HTTP/2 を受けていない（PR のセルフレビューで確認）。
- dependabot の #800（cargo-minor-patch）は 2026-11-07 に dependabot 自身が閉じていて、後継は #822 だった。
- #822 のマージ後は、残りが ctutils・rustls-webpki・h2 の 3 件になった。

## Q1: 残る h2（RUSTSEC-2026-0258）をどうするか

- 選択肢:
  - actix-web の `http2` を外す
  - 理由を付けて ignore する
  - red のまま上流の修正を待つ
- 回答: **actix-web の `http2` を外す**。api-server は LAN 内で HTTP/1.1（Vite の proxy 経由）で使うので、HTTP/2 は要らない。これで h2 が依存から消え、ignore が要らなくなる。

## Q2: 重なる dependabot の cargo の PR をどうするか

- 選択肢:
  - この PR は脆弱性の crate だけを上げ、dependabot の PR は後で取り込む
  - dependabot の PR を先にマージする
- 回答: **dependabot の PR を先にマージする**。#800 は閉じていたので、後継の #822 を、ユーザーの確認を取り直してからマージした（`add4cf13`）。
