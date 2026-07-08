import { describe, it, expect } from "vitest";
import {
  maru,
  roiPct,
  placeBand,
  jstHm,
  postMinutes,
  summaryLine,
  groupLegs,
  skipReason,
  flipNotes,
  tierBadge,
  roughnessChip,
  boardHref,
} from "./live";

describe("boardHref", () => {
  it("builds a drilldown link with from=live and date", () => {
    expect(boardHref("202602050811", "2026-07-08")).toBe(
      "/races/202602050811/board?from=live&date=2026-07-08",
    );
  });
  it("omits date when empty (盤レスポンスの date にフォールバックさせる)", () => {
    expect(boardHref("202602050811", "")).toBe(
      "/races/202602050811/board?from=live",
    );
  });
});

describe("tierBadge", () => {
  it("maps tier slugs to badges", () => {
    expect(tierBadge("buy")).toBe("🟢買い");
    expect(tierBadge("close")).toBe("🟡惜しい");
    expect(tierBadge("watch")).toBe("⚪様子見");
    expect(tierBadge("hidden")).toBe("非表示");
  });
  it("falls back to the raw slug when unknown", () => {
    expect(tierBadge("bogus")).toBe("bogus");
  });
});

describe("roughnessChip", () => {
  it("formats score with label", () => {
    expect(roughnessChip(0.88, "荒れ")).toBe("荒れ 0.88");
    expect(roughnessChip(0.5, "堅い")).toBe("堅い 0.50");
  });
  it("returns null when score or label is missing (old rows)", () => {
    expect(roughnessChip(null, "荒れ")).toBeNull();
    expect(roughnessChip(0.7, null)).toBeNull();
    expect(roughnessChip(undefined, undefined)).toBeNull();
  });
});

describe("postMinutes", () => {
  it("parses HH:MM to minutes", () => {
    expect(postMinutes("15:35")).toBe(935);
    expect(postMinutes("09:05")).toBe(545);
  });
  it("orders non-zero-padded correctly (数値比較)", () => {
    // 文字列辞書順なら "9:30" > "10:00" で壊れるが、数値化で正しく並ぶ。
    expect(postMinutes("9:30")).toBeLessThan(postMinutes("10:00"));
  });
  it("null / invalid go to the tail (+∞)", () => {
    expect(postMinutes(null)).toBe(Number.POSITIVE_INFINITY);
    expect(postMinutes("--")).toBe(Number.POSITIVE_INFINITY);
  });
});

describe("maru", () => {
  it("maps 1..20 to circled numbers", () => {
    expect(maru(1)).toBe("①");
    expect(maru(4)).toBe("④");
    expect(maru(20)).toBe("⑳");
  });
  it("falls back to plain number outside 1..20", () => {
    expect(maru(0)).toBe("0");
    expect(maru(21)).toBe("21");
  });
});

describe("roiPct", () => {
  it("no decimals for integers", () => {
    expect(roiPct(80)).toBe("80%");
    expect(roiPct(125)).toBe("125%");
  });
  it("1 decimal for fractions", () => {
    expect(roiPct(78.9)).toBe("78.9%");
  });
});

describe("placeBand", () => {
  it("formats low–high with 1 decimal", () => {
    expect(placeBand(1.1, 1.3)).toBe("1.1–1.3");
    expect(placeBand(2, 3.5)).toBe("2.0–3.5");
  });
  it("normalizes a reversed band via min/max", () => {
    expect(placeBand(1.3, 1.1)).toBe("1.1–1.3");
  });
  it("returns — when either bound is missing (JRA 未公開)", () => {
    expect(placeBand(null, 1.3)).toBe("—");
    expect(placeBand(1.1, null)).toBe("—");
    expect(placeBand(undefined, undefined)).toBe("—");
  });
});

describe("jstHm", () => {
  it("formats UTC rfc3339 to JST HH:MM", () => {
    // 06:20Z = JST 15:20
    expect(jstHm("2026-06-20T06:20:00Z")).toBe("15:20");
  });
  it("returns — for null / invalid", () => {
    expect(jstHm(null)).toBe("—");
    expect(jstHm("nonsense")).toBe("—");
  });
});

describe("summaryLine", () => {
  it("shows 張る count with watched", () => {
    expect(
      summaryLine({
        bet_race_count: 1,
        watched_race_count: 3,
        last_updated: null,
      }),
    ).toBe("🟢張る 1レース（監視中 3R）");
  });
  it("shows 張り無し when zero bets (no vague hedge)", () => {
    expect(
      summaryLine({
        bet_race_count: 0,
        watched_race_count: 3,
        last_updated: null,
      }),
    ).toBe("張り無し（監視中 3R）");
  });
});

describe("groupLegs", () => {
  it("bundles by bet_type+method, orders wide→quinella→trio", () => {
    const groups = groupLegs([
      { bet_type: "trio", method: "nagashi", axis: 4, combo: [4, 5, 7], points: 1, amount: 200 },
      { bet_type: "wide", method: "nagashi", axis: 4, combo: [4, 5], points: 1, amount: 500 },
      { bet_type: "wide", method: "nagashi", axis: 4, combo: [4, 7], points: 1, amount: 500 },
    ]);
    expect(groups.map((g) => g.betType)).toEqual(["wide", "trio"]);
    const wide = groups[0];
    expect(wide.axis).toBe(4);
    expect(wide.members).toEqual([5, 7]); // 軸を除いた相手
    expect(wide.points).toBe(2);
    expect(wide.amount).toBe(1000);
  });

  it("sums same combo within a method layer (二重購入防止)", () => {
    const groups = groupLegs([
      { bet_type: "wide", method: "nagashi", axis: 4, combo: [4, 5], points: 1, amount: 300 },
      { bet_type: "wide", method: "nagashi", axis: 4, combo: [5, 4], points: 1, amount: 200 },
    ]);
    expect(groups).toHaveLength(1);
    expect(groups[0].points).toBe(1); // 同一組番は 1 点に合算
    expect(groups[0].amount).toBe(500);
  });

  it("orders nagashi before box within the same bet_type", () => {
    const groups = groupLegs([
      { bet_type: "trio", method: "box", axis: null, combo: [4, 5, 7], points: 1, amount: 300 },
      { bet_type: "trio", method: "nagashi", axis: 4, combo: [4, 5, 8], points: 1, amount: 200 },
    ]);
    expect(groups.map((g) => g.method)).toEqual(["nagashi", "box"]);
  });

  it("keeps box and nagashi separate even for same combo", () => {
    const groups = groupLegs([
      { bet_type: "trio", method: "box", axis: null, combo: [4, 5, 7], points: 1, amount: 300 },
      { bet_type: "trio", method: "nagashi", axis: 4, combo: [4, 5, 7], points: 1, amount: 200 },
    ]);
    expect(groups).toHaveLength(2);
    const box = groups.find((g) => g.method === "box")!;
    const nagashi = groups.find((g) => g.method === "nagashi")!;
    expect(box.axis).toBeNull();
    expect(box.members).toEqual([4, 5, 7]); // 構成馬（軸なし）
    expect(nagashi.axis).toBe(4);
    expect(nagashi.members).toEqual([5, 7]); // 相手
    // 同一組番でも合算されず内訳保持
    expect(box.amount).toBe(300);
    expect(nagashi.amount).toBe(200);
  });
});

describe("skipReason", () => {
  it("plain ROI + −EV", () => {
    expect(skipReason({ roi: 78.9, axis: 9, axis_win_odds: 5.0 })).toBe(
      "ROI 78.9%・−EV",
    );
  });
  it("notes 断然人気 when odds low", () => {
    expect(skipReason({ roi: 80, axis: 2, axis_win_odds: 1.4 })).toBe(
      "◎②断然人気 単勝1.4・ROI 80%・−EV",
    );
  });
  it("断然人気 boundary: <=1.9 に入る / 2.0 は入らない", () => {
    expect(skipReason({ roi: 80, axis: 2, axis_win_odds: 1.9 })).toContain(
      "断然人気",
    );
    expect(skipReason({ roi: 80, axis: 2, axis_win_odds: 2.0 })).not.toContain(
      "断然人気",
    );
  });
});

describe("flipNotes", () => {
  it("both changes independently", () => {
    const notes = flipNotes(
      { axis_changed: true, ev_reversed: true, prev_axis: 6, prev_roi: 103, prev_verdict: "bet" },
      { axis: 9, roi: 78.9, verdict: "skip" },
    );
    expect(notes).toEqual(["+EV→−EVに反転（ROI 103%→78.9%）", "◎⑥→⑨"]);
  });
  it("ev_reversed only (軸不変)", () => {
    const notes = flipNotes(
      { axis_changed: false, ev_reversed: true, prev_axis: 4, prev_roi: 103, prev_verdict: "bet" },
      { axis: 4, roi: 78.9, verdict: "skip" },
    );
    expect(notes).toEqual(["+EV→−EVに反転（ROI 103%→78.9%）"]);
  });
  it("axis_changed only (verdict不変)", () => {
    const notes = flipNotes(
      { axis_changed: true, ev_reversed: false, prev_axis: 6, prev_roi: 120, prev_verdict: "bet" },
      { axis: 9, roi: 118, verdict: "bet" },
    );
    expect(notes).toEqual(["◎⑥→⑨"]);
  });
  it("ev_reversed in bet direction (−EV→+EV)", () => {
    const notes = flipNotes(
      { axis_changed: false, ev_reversed: true, prev_axis: 4, prev_roi: 92, prev_verdict: "skip" },
      { axis: 4, roi: 108, verdict: "bet" },
    );
    expect(notes).toEqual(["−EV→+EVに反転（ROI 92%→108%）"]);
  });
  it("no flip = empty", () => {
    const notes = flipNotes(
      { axis_changed: false, ev_reversed: false, prev_axis: 4, prev_roi: 122, prev_verdict: "bet" },
      { axis: 4, roi: 125, verdict: "bet" },
    );
    expect(notes).toEqual([]);
  });
});
