import { describe, it, expect, vi, afterEach } from "vitest";
import {
  todayJst,
  pct,
  yen,
  recoveryRate,
  VENUE_JP,
  SURFACE_JP,
  raceBadge,
} from "./format";

describe("todayJst", () => {
  afterEach(() => vi.useRealTimers());

  it("returns YYYY-MM-DD", () => {
    expect(todayJst()).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });

  it("uses JST, not UTC (UTC 夜は JST で翌日)", () => {
    vi.useFakeTimers();
    // 2026-06-14T20:00:00Z = JST 2026-06-15 05:00。UTC 日付は 06-14 だが JST は 06-15。
    vi.setSystemTime(new Date("2026-06-14T20:00:00Z"));
    expect(todayJst()).toBe("2026-06-15");
  });
});

describe("pct", () => {
  it("formats rate as percent with 1 decimal", () => {
    expect(pct(0.254)).toBe("25.4%");
    expect(pct(0)).toBe("0.0%");
    expect(pct(1)).toBe("100.0%");
  });
});

describe("yen", () => {
  it("formats with thousands separators", () => {
    expect(yen(12345)).toBe("¥12,345");
    expect(yen(0)).toBe("¥0");
  });
});

describe("recoveryRate", () => {
  it("returns percent", () => {
    expect(recoveryRate(1500, 1000)).toBe(150);
  });
  it("null when no bet", () => {
    expect(recoveryRate(0, 0)).toBeNull();
  });
});

describe("venue/surface maps", () => {
  it("maps known venue slug to JP", () => {
    expect(VENUE_JP.hakodate).toBe("函館");
    expect(VENUE_JP.hanshin).toBe("阪神");
  });
  it("has no entry for unknown slug (caller falls back to slug)", () => {
    expect(VENUE_JP.unknown).toBeUndefined();
  });
  it("maps surface", () => {
    expect(SURFACE_JP.turf).toBe("芝");
    expect(SURFACE_JP.dirt).toBe("ダ");
  });
});

describe("raceBadge", () => {
  it("bought takes precedence", () => {
    expect(
      raceBadge({ bought: true, hasSession: true, completed: true }),
    ).toBe("bought");
  });
  it("completed session + not bought = skipped", () => {
    expect(
      raceBadge({ bought: false, hasSession: true, completed: true }),
    ).toBe("skipped");
  });
  it("in-progress session + not bought = pending", () => {
    expect(
      raceBadge({ bought: false, hasSession: true, completed: false }),
    ).toBe("pending");
  });
  it("no session = none", () => {
    expect(
      raceBadge({ bought: false, hasSession: false, completed: false }),
    ).toBe("none");
  });
});
