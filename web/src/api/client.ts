import createClient from "openapi-fetch";
import type { paths, components } from "./schema";

// openapi.json の各 path が既に `/api/...` を含むため baseUrl は付けない（同一オリジン相対）。
// dev は Vite proxy、本番は nginx が同一オリジンで /api を裁く。
export const api = createClient<paths>({ baseUrl: "" });

// レスポンス型の再利用エイリアス（必要になった時点で増やす。今は分析の共通行のみ）。
export type Schemas = components["schemas"];
export type GroupStat = Schemas["GroupStatSchema"];
