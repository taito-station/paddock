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
  backParam,
  raceStarted,
  isPastDate,
  isSoon,
  sortRaces,
  filterRaces,
  parseLiveQuery,
  liveQueryParams,
  freshness,
  hasUpcomingRaces,
  DEFAULT_LIVE_QUERY,
  defaultDir,
  type LiveQuery,
} from "./live";

import type { Schemas } from "../api/client";

type LiveRaceView = Schemas["LiveRaceViewSchema"];

// テーブル型ボード（#370/#372）用の最小 LiveRaceView。指定フィールドだけ上書きする。
// 戻り値を実型にしておき、スキーマのフィールド改名を typecheck で検出させる。
function raceView(over: Partial<LiveRaceView> = {}): LiveRaceView {
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
    slip: { legs: [], race_budget: 5000 },
    flip: {
      axis_changed: false,
      prev_axis: null,
      ev_reversed: false,
      prev_verdict: null,
      prev_roi: null,
    },
    ...over,
  };
}

const DATE = "2026-07-11";
// JST 12:00 = UTC 03:00（マシン TZ 非依存を検証するため UTC 指定で固定）
const NOON = new Date("2026-07-11T03:00:00Z");

describe("boardHref", () => {
  it("builds a drilldown link with date", () => {
    expect(boardHref("202602050811", "2026-07-08")).toBe(
      "/races/202602050811/board?date=2026-07-08",
    );
  });
  it("omits date when empty (盤レスポンスの date にフォールバックさせる)", () => {
    expect(boardHref("202602050811", "")).toBe("/races/202602050811/board");
  });
  it("encodes hostile date (URL クエリ経由のユーザ制御値によるクエリ注入防止)", () => {
    expect(boardHref("202602050811", "2026-07-08&sort=roi#x")).toBe(
      "/races/202602050811/board?date=2026-07-08%26sort%3Droi%23x",
    );
  });
  it("carries back= (絞り込み状態) URL エンコードして持ち回る（#380）", () => {
    expect(boardHref("202602050811", "2026-07-08", "sort=roi&status=upcoming")).toBe(
      "/races/202602050811/board?date=2026-07-08&back=sort%3Droi%26status%3Dupcoming",
    );
  });
  it("omits back when empty (素の盤 URL を保つ)", () => {
    expect(boardHref("202602050811", "2026-07-08", "")).toBe(
      "/races/202602050811/board?date=2026-07-08",
    );
  });
  it("back のみで date 無しでも組める（直リンク相当）", () => {
    expect(boardHref("202602050811", "", "sort=roi")).toBe(
      "/races/202602050811/board?back=sort%3Droi",
    );
  });
  it("back を decode → parseLiveQuery で round-trip する（whitelist 復元）", () => {
    const q: LiveQuery = { sort: "roi", dir: "desc", status: "upcoming", verdict: "bet" };
    const href = boardHref("202602050811", "2026-07-08", backParam(q));
    // 盤が受け取る側の復元経路を再現（URL → back → LiveQuery）。
    const sp = new URLSearchParams(href.split("?")[1]);
    expect(parseLiveQuery(new URLSearchParams(sp.get("back") ?? ""))).toEqual(q);
  });
});

describe("backParam", () => {
  it("既定クエリは空文字（素の一覧に戻す）", () => {
    expect(backParam(DEFAULT_LIVE_QUERY)).toBe("");
  });
  it("既定方向の dir は省略し sort/filter のみ直列化する", () => {
    expect(
      backParam({ sort: "roi", dir: "desc", status: "upcoming", verdict: "bet" }),
    ).toBe("sort=roi&status=upcoming&verdict=bet");
  });
  it("date は含めない（盤 URL の ?date= が正本）", () => {
    expect(backParam({ ...DEFAULT_LIVE_QUERY, status: "finished" })).toBe(
      "status=finished",
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
  it("rejects loose minutes to the tail (厳格 regex に一本化・#385)", () => {
    // 分 1 桁（"12:5"）や余分な要素（"12:05:30"）は不正扱い＝+∞。
    // postDate（raceStarted 系）の厳格解釈と判定が割れないことを担保する。
    expect(postMinutes("12:5")).toBe(Number.POSITIVE_INFINITY);
    expect(postMinutes("12:05:30")).toBe(Number.POSITIVE_INFINITY);
    // 一方、厳格でも通る正常系は従来どおり分に化ける。
    expect(postMinutes("12:05")).toBe(725);
  });
  it("範囲外は末尾送り（postDate と判定を割らない・#385）", () => {
    // 時 24+・分 60+ は postDate 側でも不明扱い。postMinutes だけ有限ソート値にしない。
    expect(postMinutes("24:00")).toBe(Number.POSITIVE_INFINITY);
    expect(postMinutes("12:60")).toBe(Number.POSITIVE_INFINITY);
    // 境界（23:59）は正常。
    expect(postMinutes("23:59")).toBe(1439);
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
        server_now: "2026-07-11T03:00:00Z",
      }),
    ).toBe("🟢張る 1レース（監視中 3R）");
  });
  it("shows 張り無し when zero bets (no vague hedge)", () => {
    expect(
      summaryLine({
        bet_race_count: 0,
        watched_race_count: 3,
        last_updated: null,
        server_now: "2026-07-11T03:00:00Z",
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
  it("緩い分表記は不明(null)に倒す（postMinutes と厳格解釈を共有・#385）", () => {
    // 分 1 桁・余分な要素は postDate 側でも不明扱い。postMinutes との判定割れを防ぐ。
    expect(raceStarted(DATE, "12:5", NOON)).toBeNull();
    expect(raceStarted(DATE, "12:05:30", NOON)).toBeNull();
  });
  it("非正規形の date（エンジン依存解釈）は不明(null)に倒す", () => {
    expect(raceStarted("2026-7-11", "9:30", NOON)).toBeNull();
    expect(raceStarted("nonsense", "9:30", NOON)).toBeNull();
  });
});

describe("isPastDate", () => {
  it("開催日の 23:59 を過ぎたら過去日", () => {
    expect(isPastDate("2026-07-10", NOON)).toBe(true);
    expect(isPastDate(DATE, NOON)).toBe(false);
    expect(isPastDate("2026-07-12", NOON)).toBe(false);
  });
  it("date 不正は false（不明を過去と断定しない）", () => {
    expect(isPastDate("2026-7-10", NOON)).toBe(false);
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
    expect(sortRaces(rs, "race", "desc", ctx).map((r) => r.race_id)).toEqual(["x", "y"]);
  });
  it("axisProb desc / asc", () => {
    expect(sortRaces(races, "axisProb", "desc", ctx).map((r) => r.race_id)).toEqual(["c", "a", "b", "d"]);
    expect(sortRaces(races, "axisProb", "asc", ctx).map((r) => r.race_id)).toEqual(["d", "b", "a", "c"]);
  });
  it("status は正準の固定順で dir を反映しない（UI 側も状態列はトグルしない前提）", () => {
    expect(sortRaces(races, "status", "desc", ctx).map((r) => r.race_id)).toEqual(
      sortRaces(races, "status", "asc", ctx).map((r) => r.race_id),
    );
  });
  it("status: post 不明同士は R 番号順（NaN に依存しない明示フォールバック）", () => {
    const rs = [
      raceView({ race_id: "n2", race_no: 8, post_time: null }),
      raceView({ race_id: "n1", race_no: 2, post_time: null }),
    ];
    expect(sortRaces(rs, "status", "asc", ctx).map((r) => r.race_id)).toEqual(["n1", "n2"]);
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

describe("hasUpcomingRaces", () => {
  it("未発走（post 不明含む）が 1 件でもあれば true", () => {
    expect(hasUpcomingRaces([raceView({ post_time: "13:00" })], DATE, NOON)).toBe(true);
    expect(hasUpcomingRaces([raceView({ post_time: null })], DATE, NOON)).toBe(true);
  });
  it("全レース発走済みなら false（空配列も false）", () => {
    expect(hasUpcomingRaces([raceView({ post_time: "11:00" })], DATE, NOON)).toBe(false);
    expect(hasUpcomingRaces([], DATE, NOON)).toBe(false);
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
    const q2: LiveQuery = { sort: "post", dir: "desc", status: "all", verdict: "all" };
    expect(parseLiveQuery(liveQueryParams(q2))).toEqual(q2);
  });
  it("既定値はクエリから省略される（素の URL を保つ）", () => {
    expect(liveQueryParams(DEFAULT_LIVE_QUERY).toString()).toBe("");
    // 列の既定方向（roi は desc）と一致する dir も省略される。
    expect(liveQueryParams({ ...DEFAULT_LIVE_QUERY, sort: "roi", dir: "desc" }).toString()).toBe("sort=roi");
    expect(liveQueryParams({ ...DEFAULT_LIVE_QUERY, sort: "roi", dir: "asc" }).toString()).toBe("sort=roi&dir=asc");
  });
  it("dir 欠落は列の既定方向へ正規化（手打ち URL と UI 初回クリックを一致させる）", () => {
    expect(parseLiveQuery(new URLSearchParams("sort=roi")).dir).toBe("desc");
    expect(parseLiveQuery(new URLSearchParams("sort=post")).dir).toBe("asc");
  });
  it("status は固定順のため dir=desc を与えても asc に正規化される", () => {
    expect(parseLiveQuery(new URLSearchParams("sort=status&dir=desc")).dir).toBe("asc");
    expect(liveQueryParams({ ...DEFAULT_LIVE_QUERY, dir: "desc" }).toString()).toBe("");
  });
});

describe("freshness", () => {
  // server_now = NOON（クライアント now と一致）・fetchedAt = NOON（fetch 直後で補間 0）に固定すると、
  // 経過 = NOON − last_updated となり較正前の従来意図を保つ。SNOW/AT はその固定値（#382）。
  const SNOW = "2026-07-11T03:00:00Z"; // = NOON
  const AT = NOON.getTime();
  it("fresh within STALE_MINUTES (未発走あり)", () => {
    // NOON = 12:00 JST。3 分前更新 → fresh
    expect(freshness("2026-07-11T02:57:00Z", SNOW, true, NOON, AT)).toEqual({ label: "3分前", state: "fresh" });
    expect(freshness("2026-07-11T02:59:30Z", SNOW, true, NOON, AT)).toEqual({ label: "たった今", state: "fresh" });
  });
  it("stale beyond STALE_MINUTES when upcoming races remain", () => {
    expect(freshness("2026-07-11T02:49:00Z", SNOW, true, NOON, AT)).toEqual({ label: "11分前", state: "stale" });
  });
  it("boundary: ちょうど STALE_MINUTES は fresh、1 秒超過で stale（生 ms 比較）", () => {
    expect(freshness("2026-07-11T02:50:00Z", SNOW, true, NOON, AT).state).toBe("fresh");
    expect(freshness("2026-07-11T02:49:59Z", SNOW, true, NOON, AT)).toEqual({
      label: "10分前",
      state: "stale",
    });
  });
  it("done when no upcoming races (警告は出さない)", () => {
    expect(freshness("2026-07-11T01:00:00Z", SNOW, false, NOON, AT).state).toBe("done");
  });
  it("null / invalid last_updated with upcoming races → stale (警戒に倒す)", () => {
    expect(freshness(null, SNOW, true, NOON, AT)).toEqual({ label: "—", state: "stale" });
    expect(freshness("nonsense", SNOW, true, NOON, AT).state).toBe("stale");
  });
  it("hours label beyond 60 minutes", () => {
    expect(freshness("2026-07-11T01:00:00Z", SNOW, true, NOON, AT).label).toBe("2時間前");
  });

  // --- サーバ時刻較正（#382）---
  it("クライアント時計が進んでいても server_now 基準で経過を出す（skew で誤 stale にしない）", () => {
    // last_updated は server_now の 3 分前。クライアント now は +5 分進んでいる（fetch も同じ skew now）。
    // 較正: base = server_now − last_updated = 3分、localDelta = now − fetchedAt = 0 → 「3分前」fresh。
    // 較正前（now 基準）なら 8 分で誤って stale 手前まで膨らむが、それを起こさない。
    const skewedNow = new Date("2026-07-11T03:05:00Z");
    const fetchedAtSkewed = skewedNow.getTime();
    expect(
      freshness("2026-07-11T02:57:00Z", SNOW, true, skewedNow, fetchedAtSkewed),
    ).toEqual({ label: "3分前", state: "fresh" });
  });
  it("fetch 後は now−fetchedAt で経過を補間し閾値を跨ぐ", () => {
    // fetch 時点: server_now − last_updated = 8分（fresh）。その後クライアント now が 3 分進む
    // （now = fetchedAt + 3分）→ 8+3 = 11分 → stale。tick による補間が効くことを示す。
    const fetchedAt = new Date("2026-07-11T03:00:00Z").getTime();
    const later = new Date("2026-07-11T03:03:00Z"); // fetchedAt + 3分
    expect(
      freshness("2026-07-11T02:52:00Z", SNOW, true, later, fetchedAt),
    ).toEqual({ label: "11分前", state: "stale" });
  });
  it("server_now が null/不正ならクライアント時計にフォールバック（従来挙動）", () => {
    // フォールバック: 経過 = now − last_updated。fetchedAt は無視。now=NOON、3分前 → 「3分前」fresh。
    expect(freshness("2026-07-11T02:57:00Z", null, true, NOON, AT)).toEqual({ label: "3分前", state: "fresh" });
    expect(freshness("2026-07-11T02:57:00Z", "nonsense", true, NOON, AT).state).toBe("fresh");
  });
  it("base 負（server_now が last_updated より前）は 0 クランプで「たった今」fresh", () => {
    // 異常系だが数式の下限を固定: last_updated が server_now より後 → base<0 → max(0,…)=0。
    expect(freshness("2026-07-11T03:02:00Z", SNOW, true, NOON, AT)).toEqual({ label: "たった今", state: "fresh" });
  });
  it("localDelta 負（fetchedAt が now より未来）は前置クランプで base に等しい", () => {
    // refetch 直後に dataUpdatedAt が now より進むケース。localDelta を 0 にクランプ → 経過 = base。
    // base = server_now − last_updated = 3分。fetchedAt = now + 5分でも「3分前」を保つ（過少表示しない）。
    const future = NOON.getTime() + 5 * 60_000;
    expect(freshness("2026-07-11T02:57:00Z", SNOW, true, NOON, future)).toEqual({ label: "3分前", state: "fresh" });
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
