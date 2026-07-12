import { describe, it, expect } from "vitest";
import {
  betKey,
  effStake,
  effPayout,
  isRaceRecorded,
  totalStake,
  buildOutcomeBets,
  toAmount,
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
});
