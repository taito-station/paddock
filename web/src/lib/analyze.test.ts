import { describe, it, expect } from "vitest";
import {
  parseKind,
  isVenueSlug,
  parseAnalyzeParams,
  analyzeSearchParams,
  completeCourse,
  DEFAULT_COURSE,
  type CourseParams,
} from "./analyze";

const sp = (qs: string) => new URLSearchParams(qs);

describe("parseKind", () => {
  it("whitelist の kind を通す", () => {
    for (const k of ["horse", "jockey", "trainer", "course"]) {
      expect(parseKind(k)).toBe(k);
    }
  });
  it("不正・未指定は既定 horse", () => {
    expect(parseKind(null)).toBe("horse");
    expect(parseKind(undefined)).toBe("horse");
    expect(parseKind("")).toBe("horse");
    expect(parseKind("owner")).toBe("horse");
  });
});

describe("isVenueSlug", () => {
  it("JRA 場 slug は true", () => {
    for (const v of ["sapporo", "hakodate", "tokyo", "nakayama", "kyoto", "kokura"]) {
      expect(isVenueSlug(v)).toBe(true);
    }
  });
  it("未知・空・null は false", () => {
    expect(isVenueSlug("oi")).toBe(false); // 地方競馬場
    expect(isVenueSlug("")).toBe(false);
    expect(isVenueSlug(null)).toBe(false);
    expect(isVenueSlug(undefined)).toBe(false);
    expect(isVenueSlug("toString")).toBe(false); // prototype 汚染除け
  });
});

describe("parseAnalyzeParams", () => {
  it("name 系タブは ?q= を trim して復元", () => {
    expect(parseAnalyzeParams(sp("kind=jockey&q=%20武豊%20"))).toEqual({
      kind: "jockey",
      name: "武豊",
      course: DEFAULT_COURSE,
    });
  });
  it("既定（クエリ無し）は horse・空名・既定 course", () => {
    expect(parseAnalyzeParams(sp(""))).toEqual({
      kind: "horse",
      name: "",
      course: DEFAULT_COURSE,
    });
  });
  it("course タブは venue/distance/surface を検証付きで復元", () => {
    expect(
      parseAnalyzeParams(sp("kind=course&venue=nakayama&distance=1600&surface=dirt")),
    ).toEqual({
      kind: "course",
      name: "",
      course: { venue: "nakayama", distance: "1600", surface: "dirt" },
    });
  });
  it("course の不正値は既定へフォールバック", () => {
    expect(
      parseAnalyzeParams(sp("kind=course&venue=oi&distance=abc&surface=synthetic")),
    ).toEqual({
      kind: "course",
      name: "",
      course: { venue: "", distance: "", surface: "turf" },
    });
  });
  it("course タブでは ?q= を無視する", () => {
    expect(parseAnalyzeParams(sp("kind=course&q=無視")).name).toBe("");
  });
});

describe("analyzeSearchParams", () => {
  const str = (u: URLSearchParams) => u.toString();

  it("既定 horse・空検索は kind/q を省略、date のみ", () => {
    expect(str(analyzeSearchParams("horse", { name: "" }, "2026-07-14"))).toBe(
      "date=2026-07-14",
    );
  });
  it("name 系は kind と q（trim）を載せる", () => {
    expect(str(analyzeSearchParams("jockey", { name: "  武豊 " }, "2026-07-14"))).toBe(
      "date=2026-07-14&kind=jockey&q=%E6%AD%A6%E8%B1%8A",
    );
  });
  it("course は既定 surface(turf) を省略、venue/distance を載せる", () => {
    const c: CourseParams = { venue: "tokyo", distance: "2000", surface: "turf" };
    expect(str(analyzeSearchParams("course", { course: c }, ""))).toBe(
      "kind=course&venue=tokyo&distance=2000",
    );
  });
  it("course の dirt は明示、不正 venue/distance は省略", () => {
    const c: CourseParams = { venue: "oi", distance: "x", surface: "dirt" };
    expect(str(analyzeSearchParams("course", { course: c }, ""))).toBe(
      "kind=course&surface=dirt",
    );
  });
  it("course の submitted が null なら kind のみ", () => {
    expect(str(analyzeSearchParams("course", { course: null }, ""))).toBe("kind=course");
  });
});

describe("completeCourse", () => {
  it("会場 slug + 距離が揃えば同値を返す", () => {
    const c: CourseParams = { venue: "kyoto", distance: "1800", surface: "turf" };
    expect(completeCourse(c)).toEqual(c);
  });
  it("会場・距離が欠ける/不正なら null", () => {
    expect(completeCourse({ venue: "", distance: "1800", surface: "turf" })).toBeNull();
    expect(completeCourse({ venue: "kyoto", distance: "", surface: "turf" })).toBeNull();
    expect(completeCourse({ venue: "oi", distance: "1800", surface: "turf" })).toBeNull();
    expect(completeCourse({ venue: "kyoto", distance: "x", surface: "turf" })).toBeNull();
  });
});

describe("round-trip: parse ∘ serialize = 恒等（アクティブタブ分）", () => {
  it("name タブ", () => {
    const url = analyzeSearchParams("trainer", { name: "藤沢和雄" }, "2026-07-14");
    const back = parseAnalyzeParams(new URLSearchParams(url.toString()));
    expect(back).toEqual({ kind: "trainer", name: "藤沢和雄", course: DEFAULT_COURSE });
  });
  it("course タブ（dirt）", () => {
    const c: CourseParams = { venue: "hanshin", distance: "1200", surface: "dirt" };
    const url = analyzeSearchParams("course", { course: c }, "2026-07-14");
    const back = parseAnalyzeParams(new URLSearchParams(url.toString()));
    expect(back).toEqual({ kind: "course", name: "", course: c });
  });
  it("course タブ（既定 turf は URL 省略→parse で復元）", () => {
    const c: CourseParams = { venue: "tokyo", distance: "2000", surface: "turf" };
    const url = analyzeSearchParams("course", { course: c }, "2026-07-14");
    expect(url.get("surface")).toBeNull(); // turf は省略されている
    const back = parseAnalyzeParams(new URLSearchParams(url.toString()));
    expect(back).toEqual({ kind: "course", name: "", course: c });
  });
});
