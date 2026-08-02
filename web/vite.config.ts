import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// dev では API を別プロセス（既定 http://localhost:8080）で動かし、/api を proxy する。
// これで同一オリジン扱いになり CORS 不要（本番は nginx が同様にリバプロする）。
// proxy 先は環境変数 PADDOCK_API_TARGET で上書き可（ポート競合時の回避用）。
// `?.trim() ||` で空文字・空白のみ(`PADDOCK_API_TARGET=` / `" "`)も既定へフォールバックさせる。
const apiTarget = process.env.PADDOCK_API_TARGET?.trim() || "http://localhost:8080";
// host 未指定だと Vite 既定 `localhost` が IPv6 ループバック(`[::1]`)のみに bind され、
// 名前解決が IPv4 を先に返す環境では `http://127.0.0.1:5173/` が接続拒否になる(#569)。
// dev の loopback 用途に限るため既定は IPv4 ループバックに固定し、LAN へは露出しない。
// env PADDOCK_DEV_HOST で上書き可（例: 全 IF へ公開したいときは `true`／`0.0.0.0`）。
const devHost = process.env.PADDOCK_DEV_HOST?.trim() || "127.0.0.1";
export default defineConfig({
  plugins: [react()],
  server: {
    host: devHost,
    proxy: {
      "/api": {
        target: apiTarget,
        changeOrigin: true,
      },
    },
  },
});
