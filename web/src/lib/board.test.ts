import { describe, expect, it } from "vitest";
import {
  DEFAULT_RACE_BUDGET,
  boardBudget,
  effectiveCap,
  keepBoardPlaceholder,
  heatColor,
  heatIntensity,
  markSymbol,
  placeOddsLabel,
  sortByModelRank,
  type BoardHorse,
} from "./board";

describe("effectiveCap", () => {
  it("セッション無し（balance=null）は予算そのまま（閲覧・検討用）", () => {
    expect(effectiveCap(5000, null)).toBe(5000);
  });
  it("セッションありは残高で頭打ち", () => {
    expect(effectiveCap(5000, 3000)).toBe(3000);
    expect(effectiveCap(5000, 8000)).toBe(5000);
  });
  it("残高 0 は cap=0（執行側は縮退表示）", () => {
    expect(effectiveCap(5000, 0)).toBe(0);
  });
});

describe("boardBudget", () => {
  it("正の cap はそのまま board API へ", () => {
    expect(boardBudget(3000)).toBe(3000);
  });
  it("cap<=0 は既定予算に倒す（budget=0 はサーバが 400 を返すため）", () => {
    expect(boardBudget(0)).toBe(DEFAULT_RACE_BUDGET);
    expect(boardBudget(-100)).toBe(DEFAULT_RACE_BUDGET);
  });
});

describe("keepBoardPlaceholder", () => {
  it("同一レース（queryKey[1] が一致）のみ placeholder を維持", () => {
    expect(keepBoardPlaceholder(["board", "race-a", 5000], "race-a")).toBe(true);
    expect(keepBoardPlaceholder(["board", "race-a", 5000], "race-b")).toBe(false);
  });
  it("prevKey 無し・要素不足は維持しない（レース跨ぎ記録バグの退行防御）", () => {
    expect(keepBoardPlaceholder(undefined, "race-a")).toBe(false);
    expect(keepBoardPlaceholder(["board"], "race-a")).toBe(false);
  });
});

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
