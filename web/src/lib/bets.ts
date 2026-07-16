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

// 記録操作が可能か。セッションあり × 未完了 × このレース未記録 × mutation 非実行中。
// 金銭操作のガードなので純粋関数に置きテーブルテストで退行を検知する。
export function canRecordOutcome(opts: {
  hasSession: boolean;
  completed: boolean;
  bought: boolean;
  pending: boolean;
}): boolean {
  return opts.hasSession && !opts.completed && !opts.bought && !opts.pending;
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
// 賭け金・払戻・予算の全入力で共用するため、ここでは 100 円単位の強制はしない
// （払戻＝JRA 払戻は 10 円単位で 100 の倍数とは限らないため。単位強制は isUnit100 で個別に行う）。
export function toAmount(v: string): number {
  const n = Number(v);
  return Number.isFinite(n) && n > 0 ? Math.floor(n) : 0;
}

// 100 円単位（端数不可）か。買い方ルール「馬券は必ず 100 円単位」の判定で、
// 賭け金・予算に用いる。0 は許容（スキップ相当）。払戻は対象外（10 円単位のため）。
export function isUnit100(n: number): boolean {
  return Number.isInteger(n) && n >= 0 && n % 100 === 0;
}

// 編集後の賭け金に 100 円単位違反があるか。UI の記録ガードに使う（払戻は対象外）。
// 推奨額は build_portfolio が 100 円単位で組むため、実質は手編集の端数を検出する。
export function hasInvalidStakeUnit(
  bets: RecommendationBet[],
  edits: Edits,
): boolean {
  return bets.some((b) => !isUnit100(effStake(b, edits)));
}
