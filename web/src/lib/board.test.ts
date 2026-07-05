import { describe, expect, it } from "vitest";
import {
  heatColor,
  heatIntensity,
  markSymbol,
  placeOddsLabel,
  sortByModelRank,
  type BoardHorse,
} from "./board";

describe("markSymbol", () => {
  it("maps slugs to symbols and null to empty", () => {
    expect(markSymbol("honmei")).toBe("◎");
    expect(markSymbol("taikou")).toBe("○");
    expect(markSymbol("hoshi")).toBe("☆");
    expect(markSymbol(null)).toBe("");
    expect(markSymbol(undefined)).toBe("");
    expect(markSymbol("unknown")).toBe("");
  });
});

describe("heatIntensity", () => {
  it("is relative to field max and clamped to [0,1]", () => {
    expect(heatIntensity(0.2, 0.2)).toBe(1);
    expect(heatIntensity(0.1, 0.2)).toBeCloseTo(0.5);
    expect(heatIntensity(0.3, 0.2)).toBe(1); // clamp
    expect(heatIntensity(0.1, 0)).toBe(0); // max 0 → 0（0除算回避）
  });
});

describe("heatColor", () => {
  it("returns an hsl string that warms (hue drops) as value approaches max", () => {
    const cool = heatColor(0, 0.2); // t=0 → hue 200
    const hot = heatColor(0.2, 0.2); // t=1 → hue 30
    expect(cool).toBe("hsl(200, 70%, 26%)");
    expect(hot).toBe("hsl(30, 70%, 46%)");
  });
});

describe("placeOddsLabel", () => {
  it("formats a band or dash when missing", () => {
    expect(placeOddsLabel(1.6, 2.0)).toBe("1.6-2.0");
    expect(placeOddsLabel(null, 2.0)).toBe("-");
    expect(placeOddsLabel(1.6, null)).toBe("-");
  });
});

describe("sortByModelRank", () => {
  it("orders by model_rank asc, stable by horse_num", () => {
    const h = (num: number, rank: number): BoardHorse =>
      ({ horse_num: num, model_rank: rank }) as BoardHorse;
    const sorted = sortByModelRank([h(5, 3), h(2, 1), h(9, 1)]);
    expect(sorted.map((x) => x.horse_num)).toEqual([2, 9, 5]);
  });
});
