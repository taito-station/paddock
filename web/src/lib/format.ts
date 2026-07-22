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

// 重賞グレードの表示ラベル（race_class スラッグ→表記）。#389。
// グレード付き（重賞・L）のみ対象。open/win*/maiden 等の条件クラスは race_name 自体が
// 「響灘特別」「3歳上1勝クラス」で自己完結するため、名前へのグレード付与はしない。
export const RACE_CLASS_JP: Record<string, string> = {
  g1: "G1",
  g2: "G2",
  g3: "G3",
  listed: "L",
};

// 表示用レース名を組み立てる（#389）。重賞・L は `七夕賞(G3)` のようにグレードを付す。
// race_name が無ければ空文字（呼び出し側で条件表示のみにフォールバック）。
export function raceTitle(
  race_name?: string | null,
  race_class?: string | null,
): string {
  if (!race_name) return "";
  const grade = race_class ? RACE_CLASS_JP[race_class] : undefined;
  return grade ? `${race_name}(${grade})` : race_name;
}

// レース一覧の状態バッジ判定（表示と分離してテスト可能にする）。
//   bought=購入済み / skipped=見送り / pending=未処理(進行中) /
//   none=セッション未作成で購入状況不明
// skipped は「明示的に見送り記録した（進行中でも）」か「完了セッションで未購入」の
// いずれかで付く（#481: 見送り痕跡をサーバ保存し、リロード後も判別できるようにした）。
export type RaceBadge = "bought" | "skipped" | "pending" | "none";
export function raceBadge(opts: {
  bought: boolean;
  hasSession: boolean;
  completed: boolean;
  skipped?: boolean;
}): RaceBadge {
  if (opts.bought) return "bought";
  if (opts.skipped) return "skipped";
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
