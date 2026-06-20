import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// dev では API を別プロセス（http://localhost:8080）で動かし、/api を proxy する。
// これで同一オリジン扱いになり CORS 不要（本番は nginx が同様にリバプロする）。
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/api": {
        target: "http://localhost:8080",
        changeOrigin: true,
      },
    },
  },
});
