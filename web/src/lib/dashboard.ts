// 日次ダッシュボード（レース一覧 × ライブ EV の統合ビュー、#378）の純粋関数群。
// 突合・行レベルのソート/フィルタ・URL クエリを担う。時刻系・クエリ型は live.ts を再利用。
import type { Schemas } from "../api/client";
import { SURFACE_JP } from "./format";
import {
  liveQueryParams,
  postMinutes,
  raceStarted,
  type LiveFilter,
  type LiveQuery,
  type LiveSortCtx,
  type SortDir,
  type SortKey,
} from "./live";

type RaceSummary = Schemas["RaceSummary"];
type LiveRaceView = Schemas["LiveRaceViewSchema"];

export type DashboardRow = {
  race: RaceSummary; // 正: /api/races（DB が正本）
  live: LiveRaceView | null; // race_id 突合。snapshot 未収載は null
  bought: boolean; // セッション明細に痕跡があるか
};

// 3 ソースの突合。races が正で、live は Map で引く。live にだけ存在する race_id は捨てる
// （races API は DB 全件を返すため通常起きない。起きたら snapshot 側の異常であり一覧の正は DB）。
// 並びは races のまま（ソートは sortRows が担う）。
export function joinRaces(
  races: RaceSummary[],
  liveRaces: LiveRaceView[] | undefined,
  boughtIds: ReadonlySet<string>,
): DashboardRow[] {
  const liveById = new Map((liveRaces ?? []).map((l) => [l.race_id, l]));
  return races.map((race) => ({
    race,
    live: liveById.get(race.race_id) ?? null,
    bought: boughtIds.has(race.race_id),
  }));
}

// EV 情報（ROI・軸・荒れ・伝票・フリップ）を行に出してよいか。
// live 無しは情報が無い。tier=hidden は当日 ROI 分布の floor 未満＝「在庫は出すが
// 買いに見せない」（#344）ため数値を見せない。
export function evVisible(row: DashboardRow): boolean {
  return row.live != null && row.live.tier !== "hidden";
}

// 行の発走時刻。hidden でも post_time は無害な事実情報なので表示・ソートに使う
//（マスクするのは EV 系の数値のみ）。live 無しは null（不明）。
export function rowPostTime(row: DashboardRow): string | null {
  return row.live?.post_time ?? null;
}

// ソート。live.ts の sortRaces と同じ意味論を DashboardRow に拡張する。
// - status（既定）: 未発走（post 不明含む）→ 発走済み。各グループ内は post 昇順、
//   post 不明同士は race_num → venue slug で安定タイブレーク
// - roi / axisProb / rough: evVisible=false の行は欠落値として方向に関わらず末尾
// - post: post 不明（live 無し含む）は方向に関わらず末尾
// - race: venue slug → R 番号（live 無し行も RaceSummary から算出でき正しく混在する）
export function sortRows(
  rows: DashboardRow[],
  key: SortKey,
  dir: SortDir,
  ctx: LiveSortCtx,
): DashboardRow[] {
  const arr = [...rows];
  if (key === "status") {
    arr.sort((a, b) => {
      const fa =
        raceStarted(ctx.date, rowPostTime(a), ctx.now) === true ? 1 : 0;
      const fb =
        raceStarted(ctx.date, rowPostTime(b), ctx.now) === true ? 1 : 0;
      if (fa !== fb) return fa - fb;
      const pa = postMinutes(rowPostTime(a));
      const pb = postMinutes(rowPostTime(b));
      // 両方 post 不明（∞ === ∞）は R 番号 → 会場で明示フォールバック。
      if (pa !== pb) return pa - pb;
      if (a.race.race_num !== b.race.race_num) {
        return a.race.race_num - b.race.race_num;
      }
      // venue slug は ASCII のため、ロケール非依存の単純比較で決定性を担保する。
      return a.race.venue < b.race.venue
        ? -1
        : a.race.venue > b.race.venue
          ? 1
          : 0;
    });
    return arr;
  }
  const sign = dir === "asc" ? 1 : -1;
  const val = (r: DashboardRow): number | string | null => {
    switch (key) {
      case "post":
        return postMinutes(rowPostTime(r));
      case "roi":
        return evVisible(r) ? r.live!.roi : null;
      case "axisProb":
        return evVisible(r) ? r.live!.axis_prob : null;
      case "rough":
        return evVisible(r) ? (r.live!.roughness ?? null) : null;
      case "race":
        return `${r.race.venue}-${String(r.race.race_num).padStart(2, "0")}`;
      default:
        return null;
    }
  };
  arr.sort((a, b) => {
    const va = val(a);
    const vb = val(b);
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

// 絞り込み。status: post 不明（live 無し含む）は「未発走」側（終了と断定しない・live.ts と
// 同規約）。verdict: bet/skip 指定時、EV 非表示行（live 無し・hidden）は verdict を
// 持たない（見せない）ため除外する。
export function filterRows(
  rows: DashboardRow[],
  f: LiveFilter,
  ctx: LiveSortCtx,
): DashboardRow[] {
  return rows.filter((r) => {
    if (f.status !== "all") {
      const finished =
        raceStarted(ctx.date, rowPostTime(r), ctx.now) === true;
      if (f.status === "finished" ? !finished : finished) return false;
    }
    if (f.verdict !== "all") {
      if (!evVisible(r)) return false;
      if (r.live!.verdict !== f.verdict) return false;
    }
    return true;
  });
}

// 状態 → URL クエリ。ソート/フィルタ（既定値は省略）に date をマージする。
// liveQueryParams は他のクエリを引き継がない設計のため、date 併存はここで面倒を見る
//（旧 LiveBets の「別パラメータ追加時はマージ方式へ」の注記に対応）。
export function dashboardQueryParams(
  q: LiveQuery,
  date: string,
): URLSearchParams {
  const sp = liveQueryParams(q);
  if (date) sp.set("date", date);
  return sp;
}

// 距離馬場列の表示（"芝1200" / "ダ1700"）。未知 surface は slug をそのまま出す。
export function surfaceDistance(surface: string, distance: number): string {
  return `${SURFACE_JP[surface] ?? surface}${distance}`;
}
