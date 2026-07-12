// ライブ EV 表示用の純粋関数群（ユニットテスト対象）。日次ダッシュボード（RaceList）と
// lib/dashboard.ts から使う。EV/伝票の計算・永続化は Rust predict-watch が正本
//（#346・ADR 0064 追補）。ここは描画整形のみ。
// NOTE: sortRaces / filterRaces は旧 /live（LiveBets）専用で #378 統合後はデッドコード。
// dashboard.ts の sortRows / filterRows が正。テスト（live.test.ts）ごと後続 issue で撤去する。
import type { Schemas } from "../api/client";

type SlipLeg = Schemas["SlipLeg"];
type LiveFlip = Schemas["LiveFlip"];
type LiveSummary = Schemas["LiveSummary"];
type LiveRaceView = Schemas["LiveRaceViewSchema"];

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

// レース詳細盤（RaceBoard）への遷移先 URL を組む単一の生成源。
// date は戻り先 /?date= の復元に使う。空なら省略（RaceBoard は盤レスポンスの date に
// フォールバックできる）。旧 from=live は /live 廃止（#378）に伴い撤去 — 既存の
// from=live 付き URL は盤が単に無視する（戻り先は常に一覧で正しい）。
export function boardHref(raceId: string, date: string): string {
  // raceId / date とも URL 経由でユーザ制御可能な値になりうる（旧ルートのリダイレクトは
  // useParams のデコード済み値を渡す）ため、必ずエンコードして埋める
  //（& / # / %2F 等によるクエリ注入・パス操作・URL 破壊の防止）。
  return `/races/${encodeURIComponent(raceId)}/board${
    date ? `?date=${encodeURIComponent(date)}` : ""
  }`;
}

// 「断然人気」とみなす◎の単勝オッズ上限。この値以下は過剰人気として見送り理由に明記する
// （CLAUDE.md「断然人気は EV がマイナスになりがち」。実運用の単勝 1.4 例をカバーする保守値）。
export const DANZEN_WIN_ODDS_MAX = 1.9;

// "HH:MM"（分は 2 桁必須）を {h, m} に厳格パースする。緩い解釈（"12:5"）は不正扱いで null。
// postMinutes（ソート）と postDate（Date 化）で post_time の解釈を一本化する唯一の入口（#385）。
// 時 1〜2 桁を許すのはゼロ埋めが崩れた供給値（"9:30"）を救うため。範囲（h≤23 等）は
// 呼び出し側の用途に委ねる（postDate は Date 化で 24:00 等を NaN→null に落とす）。
function parseHm(t: string | null | undefined): { h: number; m: number } | null {
  if (!t) return null;
  const m = /^(\d{1,2}):(\d{2})$/.exec(t);
  if (!m) return null;
  return { h: Number(m[1]), m: Number(m[2]) };
}

// "HH:MM" を分に数値化する。欠落・不正は +∞（末尾送り）。文字列辞書順だとゼロ埋めが
// 崩れた供給値（"9:30"）で順序が壊れるため、時刻順を数値比較で確定させる。
export function postMinutes(t: string | null | undefined): number {
  const hm = parseHm(t);
  return hm ? hm.h * 60 + hm.m : Number.POSITIVE_INFINITY;
}

// 冒頭の一望サマリ 1 行。張る本数が 0 なら「張り無し」を明示（曖昧な据え置きをしない）。
export function summaryLine(s: LiveSummary): string {
  const head =
    s.bet_race_count > 0
      ? `🟢張る ${s.bet_race_count}レース`
      : "張り無し";
  return `${head}（監視中 ${s.watched_race_count}R）`;
}

// 段階 ROI tier のバッジ表記（買い強度）。買いは ROI≥100 のみ、以下は当日分布の相対位置（#344）。
export const TIER_BADGE: Record<string, string> = {
  buy: "🟢買い",
  close: "🟡惜しい",
  watch: "⚪様子見",
  hidden: "非表示",
};

export function tierBadge(tier: string): string {
  return TIER_BADGE[tier] ?? tier;
}

// tier の描画順位（買い強度の高い順）。ボードの一次ソートキー。未知 tier は末尾。#344
const TIER_RANK: Record<string, number> = { buy: 0, close: 1, watch: 2, hidden: 3 };
export function tierRank(tier: string): number {
  return TIER_RANK[tier] ?? 99;
}

// 荒れ度チップの表記（例 "荒れ 0.88"）。ROI（期待値）とは別軸の「分布の乱れ」。
// roughness スコア or ラベルが欠ければ null（旧データ・算出不能。チップを出さない）。#344
export function roughnessChip(
  roughness: number | null | undefined,
  label: string | null | undefined,
): string | null {
  if (roughness == null || !label) return null;
  return `${label} ${roughness.toFixed(2)}`;
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
// writer（現行は Rust predict-watch）は box×nagashi の同一組番を別 leg で保持する（内訳保存）。合算は同一
// method レイヤー内のみに閉じ、box と nagashi は別グループのまま区別を失わない（設計書「方式の付与」）。
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
      // writer は「1 leg = 1 組番 = 1 点」粒度のため両者は一致するが、同一組番の合算後は
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

// ===== テーブル型ボード（#370/#372）用の純粋関数群 =====
// now / date はすべて引数で受ける（テスト可能性とマシン TZ 非依存のため）。

// 発走 N 分前を「まもなく発走」としてハイライトする閾値（predict-watch の監視窓 40 分の半分）。
export const SOON_MINUTES = 20;
// スナップショット鮮度の警告閾値（predict-watch のスイープ間隔 5 分の 2 倍）。
export const STALE_MINUTES = 10;

// 開催日 + JST "HH:MM" を Date にする。+09:00 を明示合成しマシン TZ に依存させない。
// 欠落・不正は null（不明。終了扱いにしない）。date も正規形（YYYY-MM-DD）を検証する
// — 非正規形（"2026-7-11" 等）は ECMAScript の日付文字列仕様外でエンジン依存の解釈になるため。
function postDate(date: string, postTime: string | null | undefined): Date | null {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) return null;
  const hm = parseHm(postTime);
  if (!hm) return null;
  const h = String(hm.h).padStart(2, "0");
  const m = String(hm.m).padStart(2, "0");
  const d = new Date(`${date}T${h}:${m}:00+09:00`);
  return Number.isNaN(d.getTime()) ? null : d;
}

// 開催日が丸ごと過去か（JST の 23:59 を過ぎたか）。races が空でスナップショットから
// 判定できないときの自動ポーリング打ち切り・監視終了判定に使う。date 不正は false（不明）。
export function isPastDate(date: string, now: Date): boolean {
  return raceStarted(date, "23:59", now) === true;
}

// 発走済みか。発走時刻ちょうどは「発走済み」。post_time 不明は null。
export function raceStarted(
  date: string,
  postTime: string | null | undefined,
  now: Date,
): boolean | null {
  const t = postDate(date, postTime);
  return t == null ? null : now.getTime() >= t.getTime();
}

// まもなく発走（未発走かつ SOON_MINUTES 分以内）。post_time 不明は false。
export function isSoon(
  date: string,
  postTime: string | null | undefined,
  now: Date,
): boolean {
  const t = postDate(date, postTime);
  if (t == null) return false;
  const diffMin = (t.getTime() - now.getTime()) / 60_000;
  return diffMin > 0 && diffMin <= SOON_MINUTES;
}

// 状態列の短縮バッジ（テーブルの幅節約用。tierBadge のフル表記と対）。
const TIER_SHORT: Record<string, string> = {
  buy: "🟢張",
  close: "🟡惜",
  watch: "⚪様",
};
export function tierShort(tier: string): string {
  return TIER_SHORT[tier] ?? tier;
}

// 未発走（不明含む）のレースが残っているか。自動ポーリング継続と鮮度 done 判定の共有述語。
export function hasUpcomingRaces(
  races: LiveRaceView[],
  date: string,
  now: Date,
): boolean {
  return races.some((r) => raceStarted(date, r.post_time, now) !== true);
}

export type SortKey = "status" | "post" | "roi" | "axisProb" | "rough" | "race";
export type SortDir = "asc" | "desc";
export type LiveSortCtx = { date: string; now: Date };

// 列の初回クリック方向。数値の大きさに関心がある列（ROI・軸勝率・荒れ度）は降順スタート。
export function defaultDir(key: SortKey): SortDir {
  return key === "roi" || key === "axisProb" || key === "rough" ? "desc" : "asc";
}

function sortValue(r: LiveRaceView, key: SortKey): number | string | null {
  switch (key) {
    case "post":
      return postMinutes(r.post_time); // 欠落は +∞（下の null 末尾処理に合流）
    case "roi":
      return r.roi;
    case "axisProb":
      return r.axis_prob;
    case "rough":
      return r.roughness ?? null;
    case "race":
      // 会場 slug → R 番号。R は 2 桁ゼロ埋めで辞書順と数値順を一致させる。
      // slug（英字）順は日本語表示名の音順とは一致しないが、目的は「同一会場の
      // R をまとめて並べる」ことでありグルーピングが安定していれば足りる。
      return `${r.venue}-${String(r.race_no).padStart(2, "0")}`;
    default:
      return null;
  }
}

// ソート。既定 "status" は「未発走を発走時刻昇順で上、発走済みは下」（今なにをすべきかを先頭に）。
// status は正準の固定順で dir を反映しない（UI 側も状態列は方向トグルさせない）。
// その他の列は dir 指定でトグル。null / 欠落値は方向に関わらず常に末尾。
export function sortRaces(
  races: LiveRaceView[],
  key: SortKey,
  dir: SortDir,
  ctx: LiveSortCtx,
): LiveRaceView[] {
  const arr = [...races];
  if (key === "status") {
    arr.sort((a, b) => {
      const fa = raceStarted(ctx.date, a.post_time, ctx.now) === true ? 1 : 0;
      const fb = raceStarted(ctx.date, b.post_time, ctx.now) === true ? 1 : 0;
      if (fa !== fb) return fa - fb;
      const pa = postMinutes(a.post_time);
      const pb = postMinutes(b.post_time);
      // post 不明（+∞）は同状態グループの末尾。両方不明（∞ === ∞）は R 番号順に
      // 明示フォールバック（∞−∞=NaN の falsy に依存しない）。
      if (pa !== pb) return pa - pb;
      return a.race_no - b.race_no;
    });
    return arr;
  }
  const sign = dir === "asc" ? 1 : -1;
  arr.sort((a, b) => {
    const va = sortValue(a, key);
    const vb = sortValue(b, key);
    const missA = va == null || va === Number.POSITIVE_INFINITY;
    const missB = vb == null || vb === Number.POSITIVE_INFINITY;
    if (missA && missB) return 0;
    if (missA) return 1;
    if (missB) return -1;
    if (typeof va === "string" && typeof vb === "string") {
      // slug ベースの ASCII 文字列なので、ロケール非依存の単純比較で決定性を担保する。
      return sign * (va < vb ? -1 : va > vb ? 1 : 0);
    }
    return sign * ((va as number) - (vb as number));
  });
  return arr;
}

export type StatusFilter = "all" | "upcoming" | "finished";
export type VerdictFilter = "all" | "bet" | "skip";
export type LiveFilter = { status: StatusFilter; verdict: VerdictFilter };

// 絞り込み。post_time 不明（raceStarted=null）は「未発走」側に寄せる（終了と断定しない）。
// tier=hidden の除外（#344 の floor 非表示）は従来通り呼び出し側で先に行う。
export function filterRaces(
  races: LiveRaceView[],
  f: LiveFilter,
  ctx: LiveSortCtx,
): LiveRaceView[] {
  return races.filter((r) => {
    if (f.status !== "all") {
      const finished = raceStarted(ctx.date, r.post_time, ctx.now) === true;
      if (f.status === "finished" ? !finished : finished) return false;
    }
    if (f.verdict !== "all" && r.verdict !== f.verdict) return false;
    return true;
  });
}

export type LiveQuery = {
  sort: SortKey;
  dir: SortDir;
  status: StatusFilter;
  verdict: VerdictFilter;
};

export const DEFAULT_LIVE_QUERY: LiveQuery = {
  sort: "status",
  dir: "asc",
  status: "all",
  verdict: "all",
};

const SORT_KEYS: SortKey[] = ["status", "post", "roi", "axisProb", "rough", "race"];

// URL クエリ → 状態。不正値は既定へフォールバック（リロード・共有耐性、#370 任意要件）。
// dir は列の意味論に正規化する: status は固定順（常に asc 扱い）、dir 欠落・不正は
// その列の既定方向 defaultDir(sort)（手打ち `?sort=roi` を UI 初回クリックと一致させる）。
export function parseLiveQuery(sp: URLSearchParams): LiveQuery {
  const sortRaw = sp.get("sort") as SortKey | null;
  const sort =
    sortRaw && SORT_KEYS.includes(sortRaw) ? sortRaw : DEFAULT_LIVE_QUERY.sort;
  const dirRaw = sp.get("dir");
  const status = sp.get("status") as StatusFilter | null;
  const verdict = sp.get("verdict") as VerdictFilter | null;
  return {
    sort,
    dir:
      sort === "status"
        ? "asc"
        : dirRaw === "asc" || dirRaw === "desc"
          ? dirRaw
          : defaultDir(sort),
    status:
      status === "upcoming" || status === "finished"
        ? status
        : DEFAULT_LIVE_QUERY.status,
    verdict:
      verdict === "bet" || verdict === "skip"
        ? verdict
        : DEFAULT_LIVE_QUERY.verdict,
  };
}

// 状態 → URL クエリ。既定値は省略し、素の /live/:date を既定表示と一致させる。
// dir は列の既定方向と一致するとき省略（parseLiveQuery の正規化と対で round-trip する）。
export function liveQueryParams(q: LiveQuery): URLSearchParams {
  const sp = new URLSearchParams();
  if (q.sort !== DEFAULT_LIVE_QUERY.sort) sp.set("sort", q.sort);
  if (q.sort !== "status" && q.dir !== defaultDir(q.sort)) sp.set("dir", q.dir);
  if (q.status !== DEFAULT_LIVE_QUERY.status) sp.set("status", q.status);
  if (q.verdict !== DEFAULT_LIVE_QUERY.verdict) sp.set("verdict", q.verdict);
  return sp;
}

export type Freshness = {
  label: string; // 相対表記（"3分前" 等）
  state: "fresh" | "stale" | "done";
};

// スナップショット鮮度（#372）。stale = STALE_MINUTES 超過かつ未発走レース残存
// （= predict-watch が動いていない疑い）。未発走ゼロなら監視終了（done、警告なし）。
export function freshness(
  lastUpdated: string | null | undefined,
  hasUpcoming: boolean,
  now: Date,
): Freshness {
  let label = "—";
  let diffMs: number | null = null;
  if (lastUpdated) {
    const t = new Date(lastUpdated);
    if (!Number.isNaN(t.getTime())) {
      diffMs = Math.max(0, now.getTime() - t.getTime());
      const mins = Math.floor(diffMs / 60_000);
      label =
        mins < 1
          ? "たった今"
          : mins < 60
            ? `${mins}分前`
            : `${Math.floor(mins / 60)}時間前`;
    }
  }
  if (!hasUpcoming) return { label, state: "done" };
  // 更新時刻が読めない（null/不正）のに未発走が残る状態も警戒対象に倒す。
  // 判定は生 ms で行う（分に floor すると実効閾値が +1 分ズレるため）。
  if (diffMs == null || diffMs > STALE_MINUTES * 60_000) {
    return { label, state: "stale" };
  }
  return { label, state: "fresh" };
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
