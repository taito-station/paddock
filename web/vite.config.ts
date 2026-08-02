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
// 指定できる値は `true`（全 IF=0.0.0.0 に bind）か IP 文字列（例 `0.0.0.0`）。
// `true` は Vite が boolean で解釈する挙動なので env の文字列 `"true"` は boolean へ
// 正規化する（文字列のままだとホスト名扱いで解決に失敗する）。boolean 正規化は `true`
// のみ対応で、無効化は「未指定」（既定 127.0.0.1）を使う——`false` を渡すと Vite 既定の
// localhost 挙動に戻り #569 の IPv6-only bind が再発するため、あえて特別扱いしない。
// 未指定・空文字・空白のみ（`PADDOCK_DEV_HOST=` / `" "`）はすべて既定へフォールバック。
const rawDevHost = process.env.PADDOCK_DEV_HOST?.trim();
const devHost = rawDevHost === "true" ? true : rawDevHost || "127.0.0.1";
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
