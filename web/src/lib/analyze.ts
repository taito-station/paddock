// Analyze の URL クエリ ⇔ タブ状態の相互変換（純関数・単一生成源）。
// URL 正の既存流儀（header-date.ts / live.ts の parseLiveQuery）に倣い whitelist 検証する。
// 設計知: docs/knowledge/analyze-search-and-state.md

import { VENUE_JP } from "./format";

export type Kind = "horse" | "jockey" | "trainer" | "course";
export type NameKind = "horse" | "jockey" | "trainer";
export type Surface = "turf" | "dirt";

export const KINDS: Kind[] = ["horse", "jockey", "trainer", "course"];

// ?kind= を whitelist 検証。不正・未指定は既定 horse。
export function parseKind(raw: string | null | undefined): Kind {
  return KINDS.includes(raw as Kind) ? (raw as Kind) : "horse";
}

// VENUE_JP のキー（JRA 場 slug）か。
export function isVenueSlug(v: string | null | undefined): v is string {
  return typeof v === "string" && Object.prototype.hasOwnProperty.call(VENUE_JP, v);
}

// course タブの検索状態。distance は form の入力と揃えて文字列で持つ（"" = 未入力）。
export interface CourseParams {
  venue: string; // VENUE_JP の slug or ""
  distance: string; // 数字列 or ""
  surface: Surface;
}

export const DEFAULT_COURSE: CourseParams = {
  venue: "",
  distance: "",
  surface: "turf",
};

export interface AnalyzeState {
  kind: Kind;
  name: string; // name 系タブの確定検索語（完全一致）
  course: CourseParams;
}

const isDistance = (v: string | null | undefined): v is string =>
  typeof v === "string" && /^\d+$/.test(v);

// URL → 状態（アクティブタブ分を復元）。不正値は既定へフォールバック。
export function parseAnalyzeParams(sp: URLSearchParams): AnalyzeState {
  const kind = parseKind(sp.get("kind"));
  const name = kind === "course" ? "" : (sp.get("q") ?? "").trim();

  let course = DEFAULT_COURSE;
  if (kind === "course") {
    const venue = sp.get("venue");
    const distance = sp.get("distance");
    const surface = sp.get("surface");
    course = {
      venue: isVenueSlug(venue) ? venue : "",
      distance: isDistance(distance) ? distance : "",
      surface: surface === "dirt" ? "dirt" : "turf",
    };
  }
  return { kind, name, course };
}

// 状態 → URL。date + kind + アクティブタブの検索語を載せる。既定値・空は省略
// （parseLiveQuery と同流儀。kind=horse・surface=turf は出さない）。
export function analyzeSearchParams(
  kind: Kind,
  active: { name?: string; course?: CourseParams | null },
  date: string,
): URLSearchParams {
  const sp = new URLSearchParams();
  if (date) sp.set("date", date);
  if (kind !== "horse") sp.set("kind", kind);

  if (kind === "course") {
    const c = active.course;
    if (c) {
      if (isVenueSlug(c.venue)) sp.set("venue", c.venue);
      if (isDistance(c.distance)) sp.set("distance", c.distance);
      if (c.surface === "dirt") sp.set("surface", c.surface);
    }
  } else {
    const q = (active.name ?? "").trim();
    if (q) sp.set("q", q);
  }
  return sp;
}
