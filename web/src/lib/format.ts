// 表示用の純粋関数群（ユニットテスト対象）。

// 既定の開催日 = JST の今日。toISOString() は UTC 基準で深夜〜午前のあいだ前日に
// ズレるため使わず、Asia/Tokyo 固定でローカル日付を組み立てる。"sv-SE" ロケールは
// YYYY-MM-DD 形式を返す。
export function todayJst(): string {
  return new Date().toLocaleDateString("sv-SE", { timeZone: "Asia/Tokyo" });
}

// レート [0,1] を百分率表記にする（例 0.254 → "25.4%"）。
export function pct(rate: number): string {
  return `${(rate * 100).toFixed(1)}%`;
}

// 円表記（3 桁区切り）。例 12345 → "¥12,345"。
export function yen(n: number): string {
  return `¥${n.toLocaleString("ja-JP")}`;
}

// 回収率（%）。総賭け金 0 のときは null（0 除算回避）。
export function recoveryRate(totalPayout: number, totalBet: number): number | null {
  if (totalBet === 0) return null;
  return (totalPayout / totalBet) * 100;
}

export const SURFACE_JP: Record<string, string> = { turf: "芝", dirt: "ダ" };

// レース一覧の状態バッジ判定（表示と分離してテスト可能にする）。
//   bought=購入済み / skipped=見送り(完了セッションで未購入) / pending=未処理(進行中) /
//   none=セッション未作成で購入状況不明
export type RaceBadge = "bought" | "skipped" | "pending" | "none";
export function raceBadge(opts: {
  bought: boolean;
  hasSession: boolean;
  completed: boolean;
}): RaceBadge {
  if (opts.bought) return "bought";
  if (opts.completed) return "skipped";
  if (opts.hasSession) return "pending";
  return "none";
}

// JRA 10 場の slug→日本語。API は venue を英字スラッグで返すため表示時に変換する。
export const VENUE_JP: Record<string, string> = {
  sapporo: "札幌",
  hakodate: "函館",
  fukushima: "福島",
  niigata: "新潟",
  tokyo: "東京",
  nakayama: "中山",
  chukyo: "中京",
  kyoto: "京都",
  hanshin: "阪神",
  kokura: "小倉",
};
