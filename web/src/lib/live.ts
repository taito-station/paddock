// ライブ EV 買い目ビュー（LiveBets）の表示用純粋関数群（ユニットテスト対象）。
// EV/伝票の計算・永続化は Rust predict-watch が正本（#346・ADR 0064 追補）。ここは描画整形のみ。
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

// ◎の複勝オッズ帯を "Y.Y–Z.Z" 表記にする（#346）。low/high のどちらかでも欠ければ "—"
// （発走直前でも JRA 未公開なら欠落しうる。単勝オッズと同じ堅牢性で扱う）。
export function placeBand(
  low: number | null | undefined,
  high: number | null | undefined,
): string {
  if (low == null || high == null) return "—";
  // 帯は low≤high が前提だが、異常データで逆転しても正しい範囲を描くよう min/max で正規化する。
  return `${Math.min(low, high).toFixed(1)}–${Math.max(low, high).toFixed(1)}`;
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
    // h23 を明示し、深夜 0 時が一部 Intl 実装で "24:00" になるのを防ぐ。
    hourCycle: "h23",
  });
}

// 「断然人気」とみなす◎の単勝オッズ上限。この値以下は過剰人気として見送り理由に明記する
// （CLAUDE.md「断然人気は EV がマイナスになりがち」。実運用の単勝 1.4 例をカバーする保守値）。
export const DANZEN_WIN_ODDS_MAX = 1.9;

// "HH:MM" を分に数値化する。欠落・不正は +∞（末尾送り）。文字列辞書順だとゼロ埋めが
// 崩れた供給値（"9:30"）で順序が壊れるため、時刻順を数値比較で確定させる。
export function postMinutes(t: string | null | undefined): number {
  if (!t) return Number.POSITIVE_INFINITY;
  const [h, m] = t.split(":");
  const min = Number(h) * 60 + Number(m);
  return Number.isFinite(min) ? min : Number.POSITIVE_INFINITY;
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
      // 点数は合算後の distinct 組番数で再計算する（正本 SlipLeg.points は使わない）。
      // emit-json は「1 leg = 1 組番 = 1 点」粒度のため両者は一致するが、同一組番の合算後は
      // 点数も 1 に畳む必要があり、combos.length が現場で買う実点数になる。
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
  if (r.axis_win_odds != null && r.axis_win_odds <= DANZEN_WIN_ODDS_MAX) {
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
    // 反転方向は現サイクルの verdict から導出する。verdict は bet/skip の二値で
    // ev_reversed=前後反転が保証されるため prev_verdict と等価（三値化したら要再考）。
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
