import { describe, it, expect } from "vitest";
import {
  betKey,
  canRecordOutcome,
  effStake,
  effPayout,
  isRaceRecorded,
  totalStake,
  buildOutcomeBets,
  toAmount,
  isUnitOf,
  isUnit100,
  hasInvalidStakeUnit,
} from "./bets";
import type { RecommendationBet } from "../api/client";

const bet = (
  bet_type: string,
  combination: string,
  stake: number,
  ev = 1.5,
): RecommendationBet => ({ bet_type, combination, stake, odds: 3.0, ev });

const BETS: RecommendationBet[] = [
  bet("馬連", "1-2", 300),
  bet("ワイド", "1-3", 200),
];

describe("effStake / effPayout", () => {
  it("falls back to recommended stake, payout 0", () => {
    expect(effStake(BETS[0], {})).toBe(300);
    expect(effPayout(BETS[0], {})).toBe(0);
  });
  it("uses edit when present", () => {
    const edits = { [betKey(BETS[0])]: { stake: 500, payout: 1200 } };
    expect(effStake(BETS[0], edits)).toBe(500);
    expect(effPayout(BETS[0], edits)).toBe(1200);
  });
});

describe("totalStake", () => {
  it("sums recommended by default", () => {
    expect(totalStake(BETS, {})).toBe(500);
  });
  it("reflects edits", () => {
    expect(totalStake(BETS, { [betKey(BETS[0])]: { stake: 0, payout: 0 } })).toBe(
      200,
    );
  });
});

describe("buildOutcomeBets", () => {
  it("includes only stake > 0 legs with full fields", () => {
    const out = buildOutcomeBets(BETS, {});
    expect(out).toHaveLength(2);
    expect(out[0]).toEqual({
      bet_type: "馬連",
      combination: "1-2",
      stake: 300,
      payout: 0,
      ev: 1.5,
    });
  });
  it("all stake 0 = empty payload (skip)", () => {
    const edits = Object.fromEntries(
      BETS.map((b) => [betKey(b), { stake: 0, payout: 0 }]),
    );
    expect(buildOutcomeBets(BETS, edits)).toEqual([]);
  });
  it("carries edited payout on kept legs", () => {
    const edits = { [betKey(BETS[1])]: { stake: 200, payout: 900 } };
    const out = buildOutcomeBets(BETS, edits);
    expect(out.find((b) => b.combination === "1-3")?.payout).toBe(900);
  });
});

describe("isRaceRecorded", () => {
  const sessionBets = [
    { race_id: "2026-1-hakodate-9-1R" },
    { race_id: "2026-1-hakodate-9-1R" },
    { race_id: "2026-2-kokura-5-3R" },
  ];
  it("明細に痕跡があれば記録済み", () => {
    expect(isRaceRecorded(sessionBets, "2026-1-hakodate-9-1R")).toBe(true);
  });
  it("痕跡が無ければ未記録（スキップ記録は痕跡が残らない仕様）", () => {
    expect(isRaceRecorded(sessionBets, "2026-1-hakodate-9-12R")).toBe(false);
    expect(isRaceRecorded([], "2026-1-hakodate-9-1R")).toBe(false);
  });
});

describe("canRecordOutcome", () => {
  it("セッションあり×未完了×未記録×非実行中のみ true", () => {
    expect(
      canRecordOutcome({ hasSession: true, completed: false, bought: false, pending: false }),
    ).toBe(true);
  });
  it.each([
    ["セッション無し", { hasSession: false, completed: false, bought: false, pending: false }],
    ["完了済みセッション", { hasSession: true, completed: true, bought: false, pending: false }],
    ["記録済みレース", { hasSession: true, completed: false, bought: true, pending: false }],
    ["mutation 実行中", { hasSession: true, completed: false, bought: false, pending: true }],
  ] as const)("%s は false", (_label, opts) => {
    expect(canRecordOutcome(opts)).toBe(false);
  });
});

describe("toAmount", () => {
  it("parses positive integers", () => {
    expect(toAmount("300")).toBe(300);
  });
  it("empty / NaN / negative -> 0", () => {
    expect(toAmount("")).toBe(0);
    expect(toAmount("abc")).toBe(0);
    expect(toAmount("-100")).toBe(0);
  });
  it("floors decimals", () => {
    expect(toAmount("150.9")).toBe(150);
  });
  it("払戻等の非 100 倍数はそのまま通す（100 円単位強制は toAmount では行わない）", () => {
    expect(toAmount("720")).toBe(720);
    expect(toAmount("150")).toBe(150);
  });
});

describe("isUnitOf", () => {
  it.each([
    // [n, unit, expected]
    [0, 1000, true], // 0 はスキップ相当で許容
    [1000, 1000, true],
    [10000, 1000, true],
    [1500, 1000, false], // 下限は満たすが端数（#424 の実害ケース）
    [999, 1000, false],
    [-1000, 1000, false],
    [1000.5, 1000, false], // 非整数
    [100, 100, true],
    [150, 100, false],
  ])("isUnitOf(%d, %d) === %s", (n, unit, expected) => {
    expect(isUnitOf(n, unit)).toBe(expected);
  });
});

describe("isUnit100", () => {
  it("100 の倍数は true（0 はスキップ相当で許容）", () => {
    expect(isUnit100(0)).toBe(true);
    expect(isUnit100(100)).toBe(true);
    expect(isUnit100(5000)).toBe(true);
  });
  it("端数・負値・非整数は false", () => {
    expect(isUnit100(150)).toBe(false);
    expect(isUnit100(99)).toBe(false);
    expect(isUnit100(-100)).toBe(false);
    expect(isUnit100(100.5)).toBe(false);
  });
});

describe("hasInvalidStakeUnit", () => {
  it("推奨のまま（100 円単位）は違反なし", () => {
    expect(hasInvalidStakeUnit(BETS, {})).toBe(false);
  });
  it("手編集で端数賭け金が入ると違反", () => {
    const edits = { [betKey(BETS[0])]: { stake: 150, payout: 0 } };
    expect(hasInvalidStakeUnit(BETS, edits)).toBe(true);
  });
  it("賭け金 0（スキップ）は違反にしない。払戻の端数は賭け金判定に無関係", () => {
    const edits = {
      [betKey(BETS[0])]: { stake: 0, payout: 720 },
      [betKey(BETS[1])]: { stake: 200, payout: 240 },
    };
    expect(hasInvalidStakeUnit(BETS, edits)).toBe(false);
  });
});
