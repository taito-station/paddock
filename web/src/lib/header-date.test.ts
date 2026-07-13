import { describe, it, expect } from "vitest";
import {
  isIsoDate,
  currentHeaderDate,
  raceListHref,
  analyzeHref,
} from "./header-date";
import { todayJst } from "./format";

const sp = (qs: string) => new URLSearchParams(qs);

describe("isIsoDate", () => {
  it("YYYY-MM-DD の形だけを受け入れる（範囲検証はしない）", () => {
    expect(isIsoDate("2026-07-14")).toBe(true);
    // 形は正しいので通す（月日の範囲は検証しない＝live.ts postDate と同一方針）。
    expect(isIsoDate("2026-13-40")).toBe(true);
  });

  it("非正規形・空・null/undefined は弾く", () => {
    expect(isIsoDate("")).toBe(false);
    expect(isIsoDate(null)).toBe(false);
    expect(isIsoDate(undefined)).toBe(false);
    expect(isIsoDate("2026-7-4")).toBe(false); // ゼロ埋め無し
    expect(isIsoDate("2026/07/14")).toBe(false);
    expect(isIsoDate("07-14-2026")).toBe(false);
  });
});

describe("currentHeaderDate", () => {
  it("?date= クエリを最優先する（pathname に依らず）", () => {
    expect(currentHeaderDate(sp("date=2026-07-14"), "/")).toBe("2026-07-14");
    expect(
      currentHeaderDate(sp("date=2026-07-14"), "/sessions/2026-01-01"),
    ).toBe("2026-07-14");
  });

  it("不正な ?date= は飛ばして path param を採用する", () => {
    expect(currentHeaderDate(sp("date=bad"), "/sessions/2026-07-14")).toBe(
      "2026-07-14",
    );
  });

  it("クエリ無し + /sessions/:date は path param を採用する", () => {
    expect(currentHeaderDate(sp(""), "/sessions/2026-07-14")).toBe(
      "2026-07-14",
    );
  });

  it("path param も不正なら todayJst() へフォールバックする", () => {
    expect(currentHeaderDate(sp(""), "/sessions/bad")).toBe(todayJst());
  });

  it("不正なパーセントエンコーディングの path param は当日へフォールバックする", () => {
    expect(currentHeaderDate(sp(""), "/sessions/%")).toBe(todayJst());
  });

  it("date を持たないルートは todayJst() を返す", () => {
    expect(currentHeaderDate(sp(""), "/")).toBe(todayJst());
    expect(currentHeaderDate(sp(""), "/analyze")).toBe(todayJst());
    expect(currentHeaderDate(sp(""), "/races/abc/board")).toBe(todayJst());
  });
});

describe("raceListHref / analyzeHref", () => {
  it("選択中の date を ?date= で引き継ぐ", () => {
    expect(raceListHref("2026-07-14")).toBe("/?date=2026-07-14");
    expect(analyzeHref("2026-07-14")).toBe("/analyze?date=2026-07-14");
  });

  it("値を encode してクエリ注入を防ぐ", () => {
    expect(raceListHref("a b")).toBe("/?date=a%20b");
    expect(analyzeHref("x&y#z")).toBe("/analyze?date=x%26y%23z");
  });

  it("raceListHref → currentHeaderDate で round-trip する", () => {
    const d = "2026-07-14";
    const qs = raceListHref(d).split("?")[1];
    expect(currentHeaderDate(new URLSearchParams(qs), "/")).toBe(d);
  });
});
