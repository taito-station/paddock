// 1レース盤の表示用純関数群（ユニットテスト対象）。確率・買い目の計算はサーバ（/board）が持つ。
// ここは「見せ方」だけ: 印記号・ヒートマップ色・並び。

import type { components } from "../api/schema";

export type BoardHorse = components["schemas"]["BoardHorseSchema"];

// CLI predict / recommendations の既定 race_budget と揃える（盤と執行パネルで共有）。
export const DEFAULT_RACE_BUDGET = 5000;

// 実効上限 cap。セッション無し（balance=null）は予算がそのまま上限（閲覧・検討用）、
// セッションがあれば実弾の残高で頭打ちにする。
export function effectiveCap(applied: number, balance: number | null): number {
  return balance == null ? applied : Math.min(applied, balance);
}

// board API へ渡す予算。budget=0 はサーバが 400 を返すため、cap が 0 以下でも
// 既定予算で盤自体は描画する（買い目の執行側は cap<=0 を縮退表示で扱う）。
export function boardBudget(cap: number): number {
  return cap > 0 ? cap : DEFAULT_RACE_BUDGET;
}

// 印スラッグ → 表示記号。無印（null）は空文字。
export const MARK_SYMBOL: Record<string, string> = {
  honmei: "◎",
  taikou: "○",
  tanana: "▲",
  renge: "△",
  hoshi: "☆",
  chui: "注",
};

export function markSymbol(slug: string | null | undefined): string {
  if (!slug) return "";
  return MARK_SYMBOL[slug] ?? "";
}

// ヒートマップ強度 [0,1]。フィールド最大の勝率に対する相対値（max=0 なら 0）。
export function heatIntensity(value: number, max: number): number {
  if (!(max > 0)) return 0;
  const r = value / max;
  return Math.max(0, Math.min(1, r));
}

// 勝率ヒートマップ色（HSL）。強いほど暖色（青緑 → 橙）。ダーク背景前提の彩度・明度。
export function heatColor(value: number, max: number): string {
  const t = heatIntensity(value, max);
  // hue: 200(寒色) → 30(暖色)、明度は強いほどわずかに上げる。
  const hue = 200 - 170 * t;
  const light = 26 + 20 * t;
  return `hsl(${hue.toFixed(0)}, 70%, ${light.toFixed(0)}%)`;
}

// モデル勝率順（昇順=1位が先頭）。同順位は馬番昇順で安定。
export function sortByModelRank(horses: BoardHorse[]): BoardHorse[] {
  return [...horses].sort(
    (a, b) => a.model_rank - b.model_rank || a.horse_num - b.horse_num,
  );
}

// 複勝オッズ帯の表示（"1.6-2.0"）。未取得は "-"。
export function placeOddsLabel(
  low: number | null | undefined,
  high: number | null | undefined,
): string {
  if (low == null || high == null) return "-";
  return `${low.toFixed(1)}-${high.toFixed(1)}`;
}
