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
  tierShort,
  roughnessChip,
  boardHref,
  raceStarted,
  isSoon,
  sortRaces,
  filterRaces,
  parseLiveQuery,
  liveQueryParams,
  freshness,
  DEFAULT_LIVE_QUERY,
  defaultDir,
  type LiveQuery,
} from "./live";

// テーブル型ボード（#370/#372）用の最小 LiveRaceView。指定フィールドだけ上書きする。
function raceView(over: Record<string, unknown> = {}) {
  return {
    race_id: "2026-1-hakodate-9-1R",
    venue: "hakodate",
    race_no: 1,
    post_time: "10:00",
    captured_at: "2026-07-11T00:00:00Z",
    verdict: "skip",
    roi: 50,
    roughness: 0.5,
    roughness_label: "堅い",
    tier: "watch",
    konsen: false,
    axis: 1,
    axis_prob: 20,
    axis_win_odds: 5.0,
    axis_place_odds_low: null,
    axis_place_odds_high: null,
    odds_missing: false,
    slip: { legs: [] },
    flip: {
      axis_changed: false,
      prev_axis: null,
      ev_reversed: false,
      prev_verdict: null,
      prev_roi: null,
    },
    ...over,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
  } as any;
}

const DATE = "2026-07-11";
// JST 12:00 = UTC 03:00（マシン TZ 非依存を検証するため UTC 指定で固定）
const NOON = new Date("2026-07-11T03:00:00Z");

describe("boardHref", () => {
  it("builds a drilldown link with from=live and date", () => {
    expect(boardHref("202602050811", "2026-07-08", { fromLive: true })).toBe(
      "/races/202602050811/board?from=live&date=2026-07-08",
    );
  });
  it("omits date when empty (盤レスポンスの date にフォールバックさせる)", () => {
    expect(boardHref("202602050811", "", { fromLive: true })).toBe(
      "/races/202602050811/board?from=live",
    );
  });
  it("omits from when not from live (盤内の場内/R切替リンク)", () => {
    expect(boardHref("202602050811", "2026-07-08")).toBe(
      "/races/202602050811/board?date=2026-07-08",
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

describe("raceStarted", () => {
  it("returns true after post time, false before (JST 合成)", () => {
    expect(raceStarted(DATE, "11:59", NOON)).toBe(true);
    expect(raceStarted(DATE, "12:01", NOON)).toBe(false);
  });
  it("post time ちょうどは発走済み", () => {
    expect(raceStarted(DATE, "12:00", NOON)).toBe(true);
  });
  it("past / future dates resolve regardless of time", () => {
    expect(raceStarted("2026-07-10", "23:59", NOON)).toBe(true);
    expect(raceStarted("2026-07-12", "00:01", NOON)).toBe(false);
  });
  it("null / invalid post_time is unknown (null)", () => {
    expect(raceStarted(DATE, null, NOON)).toBeNull();
    expect(raceStarted(DATE, "--", NOON)).toBeNull();
    expect(raceStarted("", "12:00", NOON)).toBeNull();
  });
  it("accepts non-zero-padded hours (\"9:30\")", () => {
    expect(raceStarted(DATE, "9:30", NOON)).toBe(true);
  });
});

describe("isSoon", () => {
  it("true within SOON_MINUTES before post", () => {
    expect(isSoon(DATE, "12:15", NOON)).toBe(true);
    expect(isSoon(DATE, "12:20", NOON)).toBe(true);
  });
  it("false beyond the window / after post / unknown", () => {
    expect(isSoon(DATE, "12:21", NOON)).toBe(false);
    expect(isSoon(DATE, "11:59", NOON)).toBe(false);
    expect(isSoon(DATE, null, NOON)).toBe(false);
  });
});

describe("tierShort", () => {
  it("maps tiers to short badges", () => {
    expect(tierShort("buy")).toBe("🟢張");
    expect(tierShort("close")).toBe("🟡惜");
    expect(tierShort("watch")).toBe("⚪様");
  });
  it("falls back to the raw slug when unknown", () => {
    expect(tierShort("bogus")).toBe("bogus");
  });
});

describe("sortRaces", () => {
  const ctx = { date: DATE, now: NOON };
  const races = [
    raceView({ race_id: "a", venue: "kokura", race_no: 7, post_time: "13:00", roi: 90, axis_prob: 40, roughness: 0.9 }),
    raceView({ race_id: "b", venue: "hakodate", race_no: 5, post_time: "11:30", roi: 120, axis_prob: 30, roughness: null }),
    raceView({ race_id: "c", venue: "hakodate", race_no: 9, post_time: "12:30", roi: 70, axis_prob: 55, roughness: 0.4 }),
    raceView({ race_id: "d", venue: "fukushima", race_no: 3, post_time: null, roi: 100, axis_prob: 10, roughness: 0.7 }),
  ];

  it("default status sort: 未発走を発走昇順で上、発走済みは下、post 不明は未発走側の末尾", () => {
    const ids = sortRaces(races, "status", "asc", ctx).map((r) => r.race_id);
    // c(12:30 未発走) → a(13:00 未発走) → d(post 不明) → b(11:30 発走済み)
    expect(ids).toEqual(["c", "a", "d", "b"]);
  });
  it("roi desc / asc", () => {
    expect(sortRaces(races, "roi", "desc", ctx).map((r) => r.race_id)).toEqual(["b", "d", "a", "c"]);
    expect(sortRaces(races, "roi", "asc", ctx).map((r) => r.race_id)).toEqual(["c", "a", "d", "b"]);
  });
  it("rough: null は方向に関わらず末尾", () => {
    expect(sortRaces(races, "rough", "desc", ctx).map((r) => r.race_id)).toEqual(["a", "d", "c", "b"]);
    expect(sortRaces(races, "rough", "asc", ctx).map((r) => r.race_id)).toEqual(["c", "d", "a", "b"]);
  });
  it("post: 欠落は方向に関わらず末尾", () => {
    expect(sortRaces(races, "post", "asc", ctx).map((r) => r.race_id)).toEqual(["b", "c", "a", "d"]);
    expect(sortRaces(races, "post", "desc", ctx).map((r) => r.race_id)).toEqual(["a", "c", "b", "d"]);
  });
  it("race: 会場→R 番号（2 桁ゼロ埋めで 10R>9R が正しく並ぶ）", () => {
    const rs = [
      raceView({ race_id: "x", venue: "hakodate", race_no: 10 }),
      raceView({ race_id: "y", venue: "hakodate", race_no: 9 }),
    ];
    expect(sortRaces(rs, "race", "asc", ctx).map((r) => r.race_id)).toEqual(["y", "x"]);
  });
  it("axisProb desc", () => {
    expect(sortRaces(races, "axisProb", "desc", ctx).map((r) => r.race_id)).toEqual(["c", "a", "b", "d"]);
  });
  it("does not mutate the input array", () => {
    const before = races.map((r) => r.race_id);
    sortRaces(races, "roi", "desc", ctx);
    expect(races.map((r) => r.race_id)).toEqual(before);
  });
});

describe("defaultDir", () => {
  it("数値系は desc スタート、それ以外は asc", () => {
    expect(defaultDir("roi")).toBe("desc");
    expect(defaultDir("axisProb")).toBe("desc");
    expect(defaultDir("rough")).toBe("desc");
    expect(defaultDir("post")).toBe("asc");
    expect(defaultDir("race")).toBe("asc");
    expect(defaultDir("status")).toBe("asc");
  });
});

describe("filterRaces", () => {
  const ctx = { date: DATE, now: NOON };
  const races = [
    raceView({ race_id: "done-bet", post_time: "11:00", verdict: "bet" }),
    raceView({ race_id: "up-bet", post_time: "13:00", verdict: "bet" }),
    raceView({ race_id: "up-skip", post_time: "14:00", verdict: "skip" }),
    raceView({ race_id: "unknown-skip", post_time: null, verdict: "skip" }),
  ];

  it("status=upcoming: 発走済みを除外、post 不明は未発走扱い", () => {
    expect(filterRaces(races, { status: "upcoming", verdict: "all" }, ctx).map((r) => r.race_id))
      .toEqual(["up-bet", "up-skip", "unknown-skip"]);
  });
  it("status=finished: 発走済みのみ", () => {
    expect(filterRaces(races, { status: "finished", verdict: "all" }, ctx).map((r) => r.race_id))
      .toEqual(["done-bet"]);
  });
  it("verdict=bet と status の併用（未発走 かつ 張り）", () => {
    expect(filterRaces(races, { status: "upcoming", verdict: "bet" }, ctx).map((r) => r.race_id))
      .toEqual(["up-bet"]);
  });
  it("all/all は全件通す", () => {
    expect(filterRaces(races, { status: "all", verdict: "all" }, ctx)).toHaveLength(4);
  });
});

describe("parseLiveQuery / liveQueryParams", () => {
  it("空クエリは既定値", () => {
    expect(parseLiveQuery(new URLSearchParams())).toEqual(DEFAULT_LIVE_QUERY);
  });
  it("不正値は既定へフォールバック", () => {
    expect(
      parseLiveQuery(new URLSearchParams("sort=bogus&dir=up&status=zzz&verdict=maybe")),
    ).toEqual(DEFAULT_LIVE_QUERY);
  });
  it("round-trip: 非既定値は保存・復元される", () => {
    const q: LiveQuery = { sort: "roi", dir: "desc", status: "upcoming", verdict: "bet" };
    expect(parseLiveQuery(liveQueryParams(q))).toEqual(q);
  });
  it("既定値はクエリから省略される（素の URL を保つ）", () => {
    expect(liveQueryParams(DEFAULT_LIVE_QUERY).toString()).toBe("");
    expect(liveQueryParams({ ...DEFAULT_LIVE_QUERY, sort: "roi" }).toString()).toBe("sort=roi");
  });
});

describe("freshness", () => {
  it("fresh within STALE_MINUTES (未発走あり)", () => {
    // NOON = 12:00 JST。3 分前更新 → fresh
    expect(freshness("2026-07-11T02:57:00Z", true, NOON)).toEqual({ label: "3分前", state: "fresh" });
    expect(freshness("2026-07-11T02:59:30Z", true, NOON)).toEqual({ label: "たった今", state: "fresh" });
  });
  it("stale beyond STALE_MINUTES when upcoming races remain", () => {
    expect(freshness("2026-07-11T02:49:00Z", true, NOON)).toEqual({ label: "11分前", state: "stale" });
  });
  it("boundary: ちょうど STALE_MINUTES は fresh", () => {
    expect(freshness("2026-07-11T02:50:00Z", true, NOON).state).toBe("fresh");
  });
  it("done when no upcoming races (警告は出さない)", () => {
    expect(freshness("2026-07-11T01:00:00Z", false, NOON).state).toBe("done");
  });
  it("null / invalid last_updated with upcoming races → stale (警戒に倒す)", () => {
    expect(freshness(null, true, NOON)).toEqual({ label: "—", state: "stale" });
    expect(freshness("nonsense", true, NOON).state).toBe("stale");
  });
  it("hours label beyond 60 minutes", () => {
    expect(freshness("2026-07-11T01:00:00Z", true, NOON).label).toBe("2時間前");
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
