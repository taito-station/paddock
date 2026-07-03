import createClient from "openapi-fetch";
import type { paths, components } from "./schema";

// openapi.json の各 path が既に `/api/...` を含むため baseUrl は付けない（同一オリジン相対）。
// dev は Vite proxy、本番は nginx が同一オリジンで /api を裁く。
export const api = createClient<paths>({ baseUrl: "" });

// レスポンス型の再利用エイリアス（必要になった時点で増やす）。
export type Schemas = components["schemas"];
export type GroupStat = Schemas["GroupStatSchema"];
export type SessionSummary = Schemas["SessionSummaryResponse"];
export type SummaryBet = Schemas["SummaryBet"];
export type RecommendationResponse = Schemas["RecommendationResponse"];
export type RecommendationBet = Schemas["RecommendationBet"];
export type BetInput = Schemas["BetInput"];
export type LiveResponse = Schemas["LiveResponse"];
export type LiveRaceView = Schemas["LiveRaceViewSchema"];
