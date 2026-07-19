import { describe, expect, it } from "vitest";
import {
  DEFAULT_RACE_BUDGET,
  boardBudget,
  effectiveCap,
  keepBoardPlaceholder,
  heatColor,
  heatIntensity,
  markSymbol,
  snapshotClock,
  sortByModelRank,
  winOddsMove,
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

describe("sortByModelRank", () => {
  it("orders by model_rank asc, stable by horse_num", () => {
    const h = (num: number, rank: number): BoardHorse =>
      ({ horse_num: num, model_rank: rank }) as BoardHorse;
    const sorted = sortByModelRank([h(5, 3), h(2, 1), h(9, 1)]);
    expect(sorted.map((x) => x.horse_num)).toEqual([2, 9, 5]);
  });
});

describe("winOddsMove", () => {
  it("オッズ下落＝人気化（▲・妙味減）", () => {
    const m = winOddsMove(4.0, 3.0);
    expect(m?.symbol).toBe("▲");
    expect(m?.cls).toBe("odds-pop");
  });
  it("オッズ上昇＝過小人気化（△・妙味）", () => {
    const m = winOddsMove(3.0, 4.0);
    expect(m?.symbol).toBe("△");
    expect(m?.cls).toBe("odds-value");
  });
  it("変化が刻み未満（1%未満）は矢印なし", () => {
    expect(winOddsMove(4.0, 4.02)).toBeNull();
  });
  it("朝・現いずれか欠落・非正値は null（両側を対称にガード）", () => {
    expect(winOddsMove(null, 4.0)).toBeNull();
    expect(winOddsMove(4.0, undefined)).toBeNull();
    expect(winOddsMove(0, 4.0)).toBeNull();
    expect(winOddsMove(4.0, 0)).toBeNull();
  });
});

describe("snapshotClock", () => {
  it("RFC3339(UTC) を JST の HH:MM に整形", () => {
    // 00:05 UTC = 09:05 JST
    expect(snapshotClock("2026-07-19T00:05:00Z")).toBe("09:05");
  });
  it("空・不正入力は空文字（表記を壊さない）", () => {
    expect(snapshotClock(null)).toBe("");
    expect(snapshotClock(undefined)).toBe("");
    expect(snapshotClock("not-a-date")).toBe("");
  });
});
