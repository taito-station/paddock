# syntax=docker/dockerfile:1
#
# 成績取り込み（paddock-parse-pdf）を dev 環境から隔離実行するための importer イメージ（#146）。
# OCR(tesseract) が CPU を食い、取り込み中に開発機が重くなる問題を、コンテナ + CPU キャップで隔離する。
# DB は compose の postgres サービスへ接続するため bind mount は使わない。

# ---- build stage ----
# rust-toolchain.toml の 1.96.0 に合わせる。
FROM rust:1.96-slim-bookworm AS builder
# sqlx の tls-native-tls が openssl を要求する。
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
# cargo registry / target をキャッシュマウントして再ビルドを高速化する（parse-pdf のみビルド。
# sqlx::migrate! がビルド時に deployments/db/migrations を埋め込む）。target はキャッシュマウント上に
# あってレイヤに残らないため、成果物を /out へ取り出してから次ステージへ COPY する。
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release -p parse-pdf --bin paddock-parse-pdf \
    && mkdir -p /out && cp target/release/paddock-parse-pdf /out/paddock-parse-pdf

# ---- runtime stage ----
FROM debian:bookworm-slim AS runtime
# 取り込みに必要な外部ツール: mutool(mupdf-tools) と tesseract + 日本語パック(jpn)。
# JRA からの取得に使う ureq(native-tls) のため libssl3 + CA 証明書も入れる。
RUN apt-get update && apt-get install -y --no-install-recommends \
        mupdf-tools tesseract-ocr tesseract-ocr-jpn libssl3 ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /out/paddock-parse-pdf /usr/local/bin/paddock-parse-pdf
COPY deployments/importer-entrypoint.sh /usr/local/bin/importer-entrypoint.sh
# 非 root 実行（外部 PDF/OCR を扱うため最小権限に。DB 接続のみで書込先はネットワーク先 + /tmp）。
RUN chmod +x /usr/local/bin/importer-entrypoint.sh \
    && useradd --create-home --uid 10001 importer
USER importer

# 起動時 preflight（tesseract + jpn パック）は paddock-parse-pdf 自身が行う。
ENTRYPOINT ["/usr/local/bin/importer-entrypoint.sh"]
