import { describe, it, expect } from "vitest";
import type { Schemas } from "../api/client";
import {
  joinRaces,
  evVisible,
  sortRows,
  filterRows,
  dashboardQueryParams,
  backToDashboardHref,
  surfaceDistance,
  type DashboardRow,
} from "./dashboard";
import { parseLiveQuery, DEFAULT_LIVE_QUERY, type LiveQuery } from "./live";

type RaceSummary = Schemas["RaceSummary"];
type LiveRaceView = Schemas["LiveRaceViewSchema"];

const DATE = "2026-07-12";
// JST 12:00 = UTC 03:00（マシン TZ 非依存を検証するため UTC 指定で固定）
const NOON = new Date("2026-07-12T03:00:00Z");
const ctx = { date: DATE, now: NOON };

function race(over: Partial<RaceSummary> = {}): RaceSummary {
  return {
    race_id: "2026-1-hakodate-10-1R",
    date: DATE,
    venue: "hakodate",
    race_num: 1,
    distance: 1200,
    surface: "turf",
    ...over,
  };
}

function liveView(over: Partial<LiveRaceView> = {}): LiveRaceView {
  return {
    race_id: "2026-1-hakodate-10-1R",
    venue: "hakodate",
    race_no: 1,
    post_time: "10:00",
    captured_at: "2026-07-12T00:00:00Z",
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

function row(
  raceOver: Partial<RaceSummary>,
  liveOver: Partial<LiveRaceView> | null,
  bought = false,
): DashboardRow {
  const r = race(raceOver);
  return {
    race: r,
    live: liveOver ? liveView({ race_id: r.race_id, ...liveOver }) : null,
    bought,
  };
}

describe("joinRaces", () => {
  const races = [
    race({ race_id: "a", race_num: 1 }),
    race({ race_id: "b", race_num: 2 }),
    race({ race_id: "c", race_num: 3 }),
  ];
  it("race_id で突合し、snapshot 未収載は live=null", () => {
    const rows = joinRaces(races, [liveView({ race_id: "b" })], new Set());
    expect(rows.map((r) => [r.race.race_id, r.live != null])).toEqual([
      ["a", false],
      ["b", true],
      ["c", false],
    ]);
  });
  it("live にだけある race_id は捨てる（DB が正）", () => {
    const rows = joinRaces(races, [liveView({ race_id: "ghost" })], new Set());
    expect(rows).toHaveLength(3);
    expect(rows.every((r) => r.live === null)).toBe(true);
  });
  it("bought を反映し、並びは races のまま", () => {
    const rows = joinRaces(races, undefined, new Set(["b"]));
    expect(rows.map((r) => r.bought)).toEqual([false, true, false]);
    expect(rows.map((r) => r.race.race_id)).toEqual(["a", "b", "c"]);
  });
});

describe("evVisible", () => {
  it("live 無し・tier=hidden は false（#344 買いに見せない）", () => {
    expect(evVisible(row({}, null))).toBe(false);
    expect(evVisible(row({}, { tier: "hidden" }))).toBe(false);
  });
  it("buy/close/watch は true", () => {
    expect(evVisible(row({}, { tier: "buy" }))).toBe(true);
    expect(evVisible(row({}, { tier: "close" }))).toBe(true);
    expect(evVisible(row({}, { tier: "watch" }))).toBe(true);
  });
});

describe("sortRows", () => {
  const rows = [
    row({ race_id: "a", venue: "kokura", race_num: 7 }, { post_time: "13:00", roi: 90, axis_prob: 40, roughness: 0.9 }),
    row({ race_id: "b", venue: "hakodate", race_num: 5 }, { post_time: "11:30", roi: 120, axis_prob: 30, roughness: null }),
    row({ race_id: "c", venue: "hakodate", race_num: 9 }, { post_time: "12:30", roi: 70, axis_prob: 55, roughness: 0.4 }),
    // live 無し（post も EV も不明）
    row({ race_id: "d", venue: "fukushima", race_num: 3 }, null),
    // hidden（post はあるが EV はマスク）
    row({ race_id: "e", venue: "fukushima", race_num: 8 }, { post_time: "12:45", tier: "hidden", roi: 999 }),
  ];

  it("status 既定順: 未発走を発走昇順で上、post 不明は未発走側末尾、発走済みは下", () => {
    expect(sortRows(rows, "status", "asc", ctx).map((r) => r.race.race_id))
      // c(12:30) → e(12:45 hidden でも post は使う) → a(13:00) → d(不明) → b(11:30 発走済み)
      .toEqual(["c", "e", "a", "d", "b"]);
  });
  it("status: post 不明同士は R 番号 → 会場のタイブレーク", () => {
    const rs = [
      row({ race_id: "y", venue: "kokura", race_num: 3 }, null),
      row({ race_id: "x", venue: "fukushima", race_num: 3 }, null),
      row({ race_id: "z", venue: "fukushima", race_num: 2 }, null),
    ];
    expect(sortRows(rs, "status", "asc", ctx).map((r) => r.race.race_id)).toEqual(["z", "x", "y"]);
  });
  it("roi: EV 非表示行（live 無し・hidden）は方向に関わらず末尾", () => {
    expect(sortRows(rows, "roi", "desc", ctx).map((r) => r.race.race_id)).toEqual(["b", "a", "c", "d", "e"]);
    expect(sortRows(rows, "roi", "asc", ctx).map((r) => r.race.race_id)).toEqual(["c", "a", "b", "d", "e"]);
  });
  it("post: 欠落（live 無し）は方向に関わらず末尾", () => {
    expect(sortRows(rows, "post", "asc", ctx).map((r) => r.race.race_id)).toEqual(["b", "c", "e", "a", "d"]);
    expect(sortRows(rows, "post", "desc", ctx).map((r) => r.race.race_id)).toEqual(["a", "e", "c", "b", "d"]);
  });
  it("rough: null・EV 非表示行は方向に関わらず末尾", () => {
    expect(sortRows(rows, "rough", "desc", ctx).map((r) => r.race.race_id)).toEqual(["a", "c", "b", "d", "e"]);
    expect(sortRows(rows, "rough", "asc", ctx).map((r) => r.race.race_id)).toEqual(["c", "a", "b", "d", "e"]);
  });
  it("axisProb: desc/asc とも EV 非表示行は末尾", () => {
    expect(sortRows(rows, "axisProb", "desc", ctx).map((r) => r.race.race_id)).toEqual(["c", "a", "b", "d", "e"]);
    expect(sortRows(rows, "axisProb", "asc", ctx).map((r) => r.race.race_id)).toEqual(["b", "a", "c", "d", "e"]);
  });
  it("race: live 無し行も RaceSummary から正しく混在ソート", () => {
    expect(sortRows(rows, "race", "asc", ctx).map((r) => r.race.race_id))
      // fukushima-03(d) → fukushima-08(e) → hakodate-05(b) → hakodate-09(c) → kokura-07(a)
      .toEqual(["d", "e", "b", "c", "a"]);
  });
  it("入力配列を破壊しない", () => {
    const before = rows.map((r) => r.race.race_id);
    sortRows(rows, "roi", "desc", ctx);
    expect(rows.map((r) => r.race.race_id)).toEqual(before);
  });
});

describe("filterRows", () => {
  const rows = [
    row({ race_id: "done-bet" }, { post_time: "11:00", verdict: "bet" }),
    row({ race_id: "up-bet" }, { post_time: "13:00", verdict: "bet" }),
    row({ race_id: "up-skip" }, { post_time: "14:00", verdict: "skip" }),
    row({ race_id: "nolive" }, null),
    row({ race_id: "hidden-bet" }, { post_time: "13:30", verdict: "bet", tier: "hidden" }),
  ];
  it("status=upcoming: 発走済みを除外、post 不明（live 無し）は未発走扱い", () => {
    expect(
      filterRows(rows, { status: "upcoming", verdict: "all" }, ctx).map((r) => r.race.race_id),
    ).toEqual(["up-bet", "up-skip", "nolive", "hidden-bet"]);
  });
  it("status=finished: 発走済みのみ（post 不明は未発走扱いで除外）", () => {
    expect(
      filterRows(rows, { status: "finished", verdict: "all" }, ctx).map((r) => r.race.race_id),
    ).toEqual(["done-bet"]);
  });
  it("verdict=bet: EV 非表示行（live 無し・hidden）は除外", () => {
    expect(
      filterRows(rows, { status: "all", verdict: "bet" }, ctx).map((r) => r.race.race_id),
    ).toEqual(["done-bet", "up-bet"]);
  });
  it("status × verdict の併用", () => {
    expect(
      filterRows(rows, { status: "upcoming", verdict: "bet" }, ctx).map((r) => r.race.race_id),
    ).toEqual(["up-bet"]);
  });
});

describe("dashboardQueryParams", () => {
  it("既定値のみなら date だけ（素の URL を保つ）", () => {
    expect(dashboardQueryParams(DEFAULT_LIVE_QUERY, DATE).toString()).toBe(`date=${DATE}`);
    expect(dashboardQueryParams(DEFAULT_LIVE_QUERY, "").toString()).toBe("");
  });
  it("sort/filter と date が併存し、parseLiveQuery で round-trip する", () => {
    const q: LiveQuery = { sort: "roi", dir: "desc", status: "upcoming", verdict: "bet" };
    const sp = dashboardQueryParams(q, DATE);
    expect(sp.get("date")).toBe(DATE);
    expect(parseLiveQuery(sp)).toEqual(q); // date は parseLiveQuery が読まないため衝突しない
  });
});

describe("backToDashboardHref", () => {
  it("空 back は素の一覧（date のみ）に戻す", () => {
    expect(backToDashboardHref("", DATE)).toBe(`/?date=${DATE}`);
  });
  it("date も back も空なら素の /", () => {
    expect(backToDashboardHref("", "")).toBe("/");
  });
  it("back の絞り込み状態を盤の date と合成して復元する", () => {
    expect(backToDashboardHref("sort=roi&status=upcoming&verdict=bet", DATE)).toBe(
      `/?sort=roi&status=upcoming&verdict=bet&date=${DATE}`,
    );
    // 復元後を parseLiveQuery で読み直すと元の状態に一致する（round-trip）。
    const q = parseLiveQuery(
      new URLSearchParams("sort=roi&status=upcoming&verdict=bet"),
    );
    const href = backToDashboardHref("sort=roi&status=upcoming&verdict=bet", DATE);
    expect(parseLiveQuery(new URLSearchParams(href.split("?")[1]))).toEqual(q);
  });
  it("不正な back は whitelist 正規化して既定へ倒す（任意文字列を埋めない）", () => {
    // 未知 sort/status/verdict は parseLiveQuery が既定に落とすため date だけが残る。
    expect(
      backToDashboardHref("sort=bogus&status=zzz&verdict=maybe&evil=1", DATE),
    ).toBe(`/?date=${DATE}`);
  });
});

describe("surfaceDistance", () => {
  it("芝/ダの表示と未知 slug のフォールバック", () => {
    expect(surfaceDistance("turf", 1200)).toBe("芝1200");
    expect(surfaceDistance("dirt", 1700)).toBe("ダ1700");
    expect(surfaceDistance("aw", 1400)).toBe("aw1400");
  });
});
