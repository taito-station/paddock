// 日次ダッシュボード（レース一覧 × ライブ EV の統合ビュー、#378）の純粋関数群。
// 突合・行レベルのソート/フィルタ・URL クエリを担う。時刻系・クエリ型は live.ts を再利用。
import type { Schemas } from "../api/client";
import { SURFACE_JP } from "./format";
import {
  liveQueryParams,
  parseLiveQuery,
  postMinutes,
  raceStarted,
  type LiveFilter,
  type LiveQuery,
  type SortDir,
  type SortKey,
} from "./live";

type RaceSummary = Schemas["RaceSummary"];
type LiveRaceView = Schemas["LiveRaceViewSchema"];
type SummaryBet = Schemas["SummaryBet"];

export type DashboardRow = {
  race: RaceSummary; // 正: /api/races（DB が正本）
  live: LiveRaceView | null; // race_id 突合。snapshot 未収載は null
  bought: boolean; // セッション明細に痕跡があるか
};

// レースが「終了（結果確定）」か。#381 で「⚫終」判定を post_time 推定（raceStarted）から
// 着順確定（result_confirmed）へ移す一次ソース。post_time 経過でも未確定なら false（走行中/結果待ち）。
export function raceFinished(row: DashboardRow): boolean {
  return row.race.result_confirmed;
}

// 購入済みレースの的中/払戻（session bets を per-race に集計）。#381。
// hit は「そのレースの総払戻 > 0」（1 点でも払戻があれば的中表示）。返還（payout=stake）も payout>0。
export type RaceOutcome = { stake: number; payout: number; hit: boolean };

export function outcomeByRace(bets: SummaryBet[]): Map<string, RaceOutcome> {
  const m = new Map<string, RaceOutcome>();
  for (const b of bets) {
    const cur = m.get(b.race_id) ?? { stake: 0, payout: 0, hit: false };
    cur.stake += b.stake;
    cur.payout += b.payout;
    m.set(b.race_id, cur);
  }
  // hit は集計後の総払戻で確定する（1 点でも払戻があれば的中表示）。
  for (const o of m.values()) o.hit = o.payout > 0;
  return m;
}

// 自動精算ポーリングの継続条件。発走済み（post_time 経過）かつ未確定のレースが 1 件以上あるか。
// これが false になれば（＝当日の発走済みレースが全確定）ポーリングを止める（#381）。
export function hasUnsettledRaces(
  races: RaceSummary[],
  date: string,
  now: Date,
): boolean {
  return races.some(
    (r) => raceStarted(date, r.post_time, now) === true && !r.result_confirmed,
  );
}

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
  return visibleLive(row) != null;
}

// EV を見せてよい行の live（narrowing ヘルパ。boolean 判定だと呼び出し側で
// non-null assertion が要るため、null 込みで返して ?. で辿らせる）。
export function visibleLive(row: DashboardRow): LiveRaceView | null {
  return row.live != null && row.live.tier !== "hidden" ? row.live : null;
}

// 行の発走時刻。races API（race_cards 正本）を一次ソースにし、live（snapshot 時点の複写）は
// fallback（#391。watch 未判定レースでも発走時刻・状態を出す）。hidden でも post_time は
// 無害な事実情報なので表示・ソートに使う（マスクするのは EV 系の数値のみ）。
export function rowPostTime(row: DashboardRow): string | null {
  return row.race.post_time ?? row.live?.post_time ?? null;
}

// DashboardRow のソート。発走時刻・状態にもとづく意味論は以下の通り。
// - status（既定）: 未発走（post 不明含む）→ 発走済み。各グループ内は post 昇順、
//   post 不明同士は race_num → venue slug で安定タイブレーク
// - roi / axisProb / rough: evVisible=false の行は欠落値として方向に関わらず末尾
// - post: post 不明（live 無し含む）は方向に関わらず末尾
// - race: venue slug → R 番号（live 無し行も RaceSummary から算出でき正しく混在する）
// #381 で status 判定を post_time 推定から result_confirmed へ移したため、date/now（旧 ctx）は不要。
export function sortRows(
  rows: DashboardRow[],
  key: SortKey,
  dir: SortDir,
): DashboardRow[] {
  const arr = [...rows];
  if (key === "status") {
    arr.sort((a, b) => {
      // 「終了」は結果確定（#381）。未発走・走行中（post 経過だが未確定）は上、確定は下。
      const fa = raceFinished(a) ? 1 : 0;
      const fb = raceFinished(b) ? 1 : 0;
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
        return visibleLive(r)?.roi ?? null;
      case "axisProb":
        return visibleLive(r)?.axis_prob ?? null;
      case "rough":
        return visibleLive(r)?.roughness ?? null;
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
export function filterRows(rows: DashboardRow[], f: LiveFilter): DashboardRow[] {
  return rows.filter((r) => {
    if (f.status !== "all") {
      // 「終了」は結果確定（#381）。post_time 経過でも未確定は「未発走」側に残す（走行中/結果待ち）。
      const finished = raceFinished(r);
      if (f.status === "finished" ? !finished : finished) return false;
    }
    if (f.verdict !== "all") {
      const live = visibleLive(r);
      if (live == null || live.verdict !== f.verdict) return false;
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

// 盤の「← レース一覧」戻り先を組む（#380）。盤 URL の back=（ライブの絞り込み状態）を
// 盤の date と合成して /?… を復元する。back は searchParams 由来のユーザ制御値なので
// parseLiveQuery で再検証し、whitelist 値のみ復元する（不正な back は既定へ正規化）。
export function backToDashboardHref(back: string, date: string): string {
  const q = parseLiveQuery(new URLSearchParams(back));
  const qs = dashboardQueryParams(q, date).toString();
  return qs ? `/?${qs}` : "/";
}

// 距離馬場列の表示（"芝1200" / "ダ1700"）。未知 surface は slug をそのまま出す。
export function surfaceDistance(surface: string, distance: number): string {
  return `${SURFACE_JP[surface] ?? surface}${distance}`;
}
