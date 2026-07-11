import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// dev では API を別プロセス（既定 http://localhost:8080）で動かし、/api を proxy する。
// これで同一オリジン扱いになり CORS 不要（本番は nginx が同様にリバプロする）。
// proxy 先は環境変数 PADDOCK_API_TARGET で上書き可（ポート競合時の回避用）。
const apiTarget = process.env.PADDOCK_API_TARGET ?? "http://localhost:8080";
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/api": {
        target: apiTarget,
        changeOrigin: true,
      },
    },
  },
});
