import createClient from "openapi-fetch";
import type { paths, components } from "./schema";

// openapi.json の各 path が既に `/api/...` を含むため baseUrl は付けない（同一オリジン相対）。
// dev は Vite proxy、本番は nginx が同一オリジンで /api を裁く。
export const api = createClient<paths>({ baseUrl: "" });

// レスポンス型の再利用エイリアス（各ビューはここから引く）。
export type Schemas = components["schemas"];
export type RaceSummary = Schemas["RaceSummary"];
export type RaceListResponse = Schemas["RaceListResponse"];
export type SessionSummaryResponse = Schemas["SessionSummaryResponse"];
export type GroupStat = Schemas["GroupStatSchema"];
export type HorseStats = Schemas["HorseStatsResponse"];
export type JockeyStats = Schemas["JockeyStatsResponse"];
export type TrainerStats = Schemas["TrainerStatsResponse"];
export type CourseStats = Schemas["CourseStatsResponse"];
