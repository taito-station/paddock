// レース詳細の買い目編集ロジック（純粋関数・テスト対象）。
// 推奨（RecommendationBet）に対するユーザー編集（賭け金/払戻）を重ね、
// outcome 記録用の BetInput[] を組み立てる。
import type { BetInput, RecommendationBet } from "../api/client";

export type Edit = { stake: number; payout: number };
export type Edits = Record<string, Edit>;

export const betKey = (b: RecommendationBet) => `${b.bet_type}-${b.combination}`;

// 編集があればその賭け金、無ければ推奨額。
export function effStake(b: RecommendationBet, edits: Edits): number {
  return edits[betKey(b)]?.stake ?? b.stake;
}

// 編集があればその払戻、無ければ 0（未確定）。
export function effPayout(b: RecommendationBet, edits: Edits): number {
  return edits[betKey(b)]?.payout ?? 0;
}

// 賭け金合計（残高超過判定に使う）。
export function totalStake(bets: RecommendationBet[], edits: Edits): number {
  return bets.reduce((s, b) => s + effStake(b, edits), 0);
}

// このレースを記録済みか（セッション明細に痕跡があるか）。スキップ記録は痕跡が残らない。
export function isRaceRecorded(
  bets: { race_id: string }[],
  raceId: string,
): boolean {
  return bets.some((b) => b.race_id === raceId);
}

// outcome 記録の payload。賭け金 > 0 の脚のみ送る（空配列 = スキップ相当）。
export function buildOutcomeBets(
  bets: RecommendationBet[],
  edits: Edits,
): BetInput[] {
  return bets
    .filter((b) => effStake(b, edits) > 0)
    .map((b) => ({
      bet_type: b.bet_type,
      combination: b.combination,
      stake: effStake(b, edits),
      payout: effPayout(b, edits),
      ev: b.ev,
    }));
}

// 数値入力のサニタイズ。空文字・NaN・負値は 0 に倒し、整数（円）に丸める。
export function toAmount(v: string): number {
  const n = Number(v);
  return Number.isFinite(n) && n > 0 ? Math.floor(n) : 0;
}
