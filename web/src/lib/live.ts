// ライブ EV 買い目ビュー（LiveBets）の表示用純粋関数群（ユニットテスト対象）。
// EV/伝票の計算ロジックは Python live_ev.py が正本（ADR 0064）。ここは描画整形のみ。
import type { Schemas } from "../api/client";

type SlipLeg = Schemas["SlipLeg"];
type LiveFlip = Schemas["LiveFlip"];
type LiveSummary = Schemas["LiveSummary"];

// 馬番を丸数字にする（①..⑳）。範囲外はそのまま数字（JRA は最大 18 のため実質全域）。
export function maru(n: number): string {
  return Number.isInteger(n) && n >= 1 && n <= 20
    ? String.fromCodePoint(0x2460 + n - 1)
    : String(n);
}

// ROI[%] 表記。整数は小数を出さず、端数があるときだけ 1 桁（80%→"80%" / 78.9→"78.9%"）。
export function roiPct(n: number): string {
  return `${Number.isInteger(n) ? n.toFixed(0) : n.toFixed(1)}%`;
}

// UTC rfc3339 の時刻を JST の HH:MM にする。null/不正は "—"。
export function jstHm(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "—";
  return d.toLocaleTimeString("ja-JP", {
    timeZone: "Asia/Tokyo",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

// 冒頭の一望サマリ 1 行。張る本数が 0 なら「張り無し」を明示（曖昧な据え置きをしない）。
export function summaryLine(s: LiveSummary): string {
  const head =
    s.bet_race_count > 0
      ? `🟢張る ${s.bet_race_count}レース`
      : "張り無し";
  return `${head}（監視中 ${s.watched_race_count}R）`;
}

export const BET_TYPE_JP: Record<string, string> = {
  wide: "ワイド",
  quinella: "馬連",
  trio: "3連複",
};

export const METHOD_JP: Record<string, string> = {
  nagashi: "ながし",
  box: "ボックス",
  formation: "フォーメーション",
};

// 券種・方式の描画順（ワイド→馬連→3連複、各内で ながし→ボックス）。
const BET_TYPE_ORDER: Record<string, number> = { wide: 0, quinella: 1, trio: 2 };
const METHOD_ORDER: Record<string, number> = { nagashi: 0, box: 1, formation: 2 };

// 「そのまま買える形」1 行分。券種×方式で束ね、同一組番の金額は合算済み。
export type LegGroup = {
  betType: string;
  method: string;
  axis: number | null; // ながし=◎馬番 / ボックス=null
  members: number[]; // ながし=相手（軸を除く union）/ ボックス=構成馬 union（昇順）
  points: number; // 組番数（合算後の distinct 組）
  amount: number; // 合計金額（円）
};

function uniqSorted(xs: number[]): number[] {
  return [...new Set(xs)].sort((a, b) => a - b);
}

// slip.legs を「券種×方式」で束ね、同一組番の金額を合算して描画用に整形する。
// emit-json は box×nagashi の同一組番を別 leg で保持する（内訳保存）。合算は同一 method
// レイヤー内のみに閉じ、box と nagashi は別グループのまま区別を失わない（設計書「方式の付与」）。
export function groupLegs(legs: SlipLeg[]): LegGroup[] {
  type Acc = {
    betType: string;
    method: string;
    axis: number | null;
    combos: Map<string, { combo: number[]; amount: number }>;
  };
  const map = new Map<string, Acc>();
  for (const leg of legs) {
    const key = `${leg.bet_type}|${leg.method}`;
    let g = map.get(key);
    if (!g) {
      g = {
        betType: leg.bet_type,
        method: leg.method,
        axis: leg.axis ?? null,
        combos: new Map(),
      };
      map.set(key, g);
    }
    if (g.axis == null && leg.axis != null) g.axis = leg.axis;
    const combo = [...leg.combo].sort((a, b) => a - b);
    const ck = combo.join(",");
    const hit = g.combos.get(ck);
    if (hit) hit.amount += leg.amount;
    else g.combos.set(ck, { combo, amount: leg.amount });
  }

  const groups: LegGroup[] = [];
  for (const g of map.values()) {
    const combos = [...g.combos.values()];
    const isBox = g.method === "box" || g.axis == null;
    const members = isBox
      ? uniqSorted(combos.flatMap((c) => c.combo))
      : uniqSorted(combos.flatMap((c) => c.combo.filter((h) => h !== g.axis)));
    groups.push({
      betType: g.betType,
      method: g.method,
      axis: isBox ? null : g.axis,
      members,
      points: combos.length,
      amount: combos.reduce((s, c) => s + c.amount, 0),
    });
  }

  return groups.sort((a, b) => {
    const bt = (BET_TYPE_ORDER[a.betType] ?? 99) - (BET_TYPE_ORDER[b.betType] ?? 99);
    if (bt !== 0) return bt;
    return (METHOD_ORDER[a.method] ?? 99) - (METHOD_ORDER[b.method] ?? 99);
  });
}

// 見送り理由の文字列。ROI と（断然人気なら）単勝オッズを添える。フリップ注記は別（flipNotes）。
export function skipReason(r: {
  roi: number;
  axis: number;
  axis_win_odds?: number | null;
}): string {
  const bits: string[] = [];
  if (r.axis_win_odds != null && r.axis_win_odds <= 1.9) {
    bits.push(`◎${maru(r.axis)}断然人気 単勝${r.axis_win_odds.toFixed(1)}`);
  }
  bits.push(`ROI ${roiPct(r.roi)}`, "−EV");
  return bits.join("・");
}

// フリップ注記。axis_changed / ev_reversed を独立に評価し、真の側のみ返す（片側 false を誤強調しない）。
export function flipNotes(
  flip: LiveFlip,
  cur: { axis: number; roi: number; verdict: string },
): string[] {
  const notes: string[] = [];
  if (flip.ev_reversed) {
    const dir = cur.verdict === "bet" ? "−EV→+EVに反転" : "+EV→−EVに反転";
    const roiPart =
      flip.prev_roi != null
        ? `（ROI ${roiPct(flip.prev_roi)}→${roiPct(cur.roi)}）`
        : "";
    notes.push(dir + roiPart);
  }
  if (flip.axis_changed) {
    const prev = flip.prev_axis != null ? maru(flip.prev_axis) : "?";
    notes.push(`◎${prev}→${maru(cur.axis)}`);
  }
  return notes;
}
