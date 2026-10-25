// レース詳細の買い目編集ロジック（純粋関数・テスト対象）。
// 推奨（RecommendationBet）に対するユーザー編集（賭け金/払戻）を重ね、
// outcome 記録用の BetInput[] を組み立てる。
import type { BetInput, RecommendationBet } from "../api/client";

export type Edit = { stake: number; payout: number };
export type Edits = Record<string, Edit>;

type BetLike = { bet_type: string; combination: string };

export const betKey = (b: BetLike) => `${b.bet_type}-${b.combination}`;

// 脚ごとの一意なキー（React の key と edits のキー）。混戦の配分は三連複ボックスと ◎軸ながしを
// 両方買うので、同じ券種・組が 2 脚入る（#772）。2 本目以降は `#2` のように出現順を付けて区別する
// （combination は数字と `-` だけなので `#` 付きのキーが実在の脚とぶつかることは無い）。
// - 脚の並びは build_portfolio のレイヤー順（ながし → ボックス）で固定なので、並び替えでは付け替わらない。
// - 朝の選定が無いレースでは、ライブ更新で相手が入れ替わり、同じ組のながし脚が増減することがある。
//   そのときは `#n` がずれて、同じ組の編集がながし脚とボックス脚のあいだで付け替わる。記録される
//   （券種・組・金額）は同じ組への編集のままで、修正前の「同じ組なら編集が持ち越される」性質と同じ。
//   方式入りのキーにするには API に method を出す必要があり、今回はそこまで広げない（#772）。
export function legKeys(bets: BetLike[]): string[] {
  const seen = new Map<string, number>();
  return bets.map((b) => {
    const k = betKey(b);
    const n = (seen.get(k) ?? 0) + 1;
    seen.set(k, n);
    return n === 1 ? k : `${k}#${n}`;
  });
}

// 編集があればその賭け金、無ければ推奨額。key は legKeys の脚キー（同じ組の 2 本目を取り違えないよう必須）。
export function effStake(b: RecommendationBet, edits: Edits, key: string): number {
  return edits[key]?.stake ?? b.stake;
}

// 編集があればその払戻、無ければ 0（未確定）。
export function effPayout(edits: Edits, key: string): number {
  return edits[key]?.payout ?? 0;
}

// 全脚を 0 円にする編集（「スキップ」ボタン）。同じ組の 2 脚とも 0 にするため脚キーで作る。
export function skipAllEdits(bets: BetLike[]): Edits {
  return Object.fromEntries(legKeys(bets).map((k) => [k, { stake: 0, payout: 0 }]));
}

// 賭け金合計（残高超過判定に使う）。
export function totalStake(bets: RecommendationBet[], edits: Edits): number {
  const keys = legKeys(bets);
  return bets.reduce((s, b, i) => s + effStake(b, edits, keys[i]), 0);
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
  const keys = legKeys(bets);
  return bets
    .map((b, i) => ({ b, key: keys[i] }))
    .filter(({ b, key }) => effStake(b, edits, key) > 0)
    .map(({ b, key }) => ({
      bet_type: b.bet_type,
      combination: b.combination,
      stake: effStake(b, edits, key),
      payout: effPayout(edits, key),
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

// n が unit 円単位（端数不可）か。金銭ガード（賭け金・予算/R の 100 円単位、セッション総予算の
// 1000 円単位）に共用する純関数。0 は許容（スキップ相当）。unit は正の整数のみ有効
// （unit<=0 は誤用として false を返す）。
export function isUnitOf(n: number, unit: number): boolean {
  return unit > 0 && Number.isInteger(n) && n >= 0 && n % unit === 0;
}

// 100 円単位（端数不可）か。買い方ルール「馬券は必ず 100 円単位」の判定で、賭け金・レース予算
// （100 円単位）に用いる。セッション総予算（1000 円単位）は isUnitOf(n, 1000) を直接使う。
// 0 は許容（スキップ相当）。払戻は対象外（10 円単位のため）。
export function isUnit100(n: number): boolean {
  return isUnitOf(n, 100);
}

// 編集後の賭け金に 100 円単位違反があるか。UI の記録ガードに使う（払戻は対象外）。
// 推奨額は build_portfolio が 100 円単位で組むため、実質は手編集の端数を検出する。
export function hasInvalidStakeUnit(
  bets: RecommendationBet[],
  edits: Edits,
): boolean {
  const keys = legKeys(bets);
  return bets.some((b, i) => !isUnit100(effStake(b, edits, keys[i])));
}
