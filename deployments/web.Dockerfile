# syntax=docker/dockerfile:1
#
# Web SPA（#34）を配信するためのコンテナイメージ。
# ビルド済み静的資産を nginx で配信し、/api は同一オリジンで api サービスへリバプロする
# （これで CORS 不要。dev は Vite proxy が同役）。
# build context はリポジトリルート（compose の web サービスで context: .. を指定）。

# ---- build stage ----
FROM node:22-slim AS builder
WORKDIR /web
# 依存だけ先に入れてレイヤキャッシュを効かせる。
COPY web/package.json web/package-lock.json ./
RUN npm ci
# ソースをコピーしてビルド（schema.d.ts はコミット済みなので gen:api は不要）。
COPY web/ ./
RUN npm run build

# ---- runtime stage ----
FROM nginx:1.27-alpine AS runtime
COPY deployments/web.nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=builder /web/dist /usr/share/nginx/html
EXPOSE 80
