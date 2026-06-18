# syntax=docker/dockerfile:1
#
# REST API（api-server, #33）を常駐実行するためのコンテナイメージ（#35）。
# DB は compose の postgres サービスへ接続するため bind mount は使わない。

# ---- build stage ----
# rust:1.96 系イメージ。正確なパッチ版（1.96.0）は COPY した rust-toolchain.toml を
# rustup が強制するため、再現性はイメージのタグではなく rust-toolchain.toml が担保する。
FROM rust:1.96-slim-bookworm AS builder
# sqlx の tls-native-tls が openssl を要求する。utoipa-swagger-ui の build script は
# Swagger UI 資産を curl で取得する（https のため CA 証明書も要る）。
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
# cargo registry / target をキャッシュマウントして再ビルドを高速化する（api-server のみビルド。
# sqlx::migrate! がビルド時に deployments/db/migrations を埋め込む）。target はキャッシュマウント上に
# あってレイヤに残らないため、成果物を /out へ取り出してから次ステージへ COPY する。
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release -p api-server --bin paddock-api \
    && mkdir -p /out && cp target/release/paddock-api /out/paddock-api

# ---- runtime stage ----
FROM debian:bookworm-slim AS runtime
# odds/netkeiba のライブスクレイプ（ureq native-tls）と sqlx native-tls のため
# libssl3 + CA 証明書を入れる（OCR は使わないので mutool/tesseract は不要）。
RUN apt-get update && apt-get install -y --no-install-recommends \
        libssl3 ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /out/paddock-api /usr/local/bin/paddock-api
# 非 root 実行（最小権限。DB 接続と外部スクレイプのみで、ローカル書込先は /tmp 程度）。
RUN useradd --create-home --uid 10001 apiserver
USER apiserver
# REST API の listen ポート。コンテナ外へ公開するには PADDOCK_SERVER_ADDR=0.0.0.0:8080 で
# bind する（既定の 127.0.0.1 はコンテナ内ループバックのみで、公開ポートから到達できない）。
EXPOSE 8080
# 起動時に自身が pool::migrate でマイグレーションを適用する（compose の depends_on で postgres 健全化を待つ）。
ENTRYPOINT ["/usr/local/bin/paddock-api"]
