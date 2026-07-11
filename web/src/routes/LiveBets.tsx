import { useEffect, useMemo, useState } from "react";
import { useParams, Link, useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { api, type LiveRaceView } from "../api/client";
import { VENUE_JP, yen } from "../lib/format";
import {
  BET_TYPE_JP,
  DANZEN_WIN_ODDS_MAX,
  METHOD_JP,
  STALE_MINUTES,
  boardHref,
  defaultDir,
  filterRaces,
  flipNotes,
  freshness,
  groupLegs,
  hasUpcomingRaces,
  isSoon,
  jstHm,
  liveQueryParams,
  maru,
  parseLiveQuery,
  placeBand,
  raceStarted,
  roiPct,
  roughnessChip,
  skipReason,
  sortRaces,
  summaryLine,
  tierShort,
  type LiveQuery,
  type SortKey,
  type StatusFilter,
  type VerdictFilter,
} from "../lib/live";

const venueJp = (v: string) => VENUE_JP[v] ?? v;
const raceLabel = (r: LiveRaceView) => `${venueJp(r.venue)}${r.race_no}R`;

// レース名を全頭盤（RaceBoard）へのドリルダウンリンクにする。#344
function RaceLabelLink({ race, date }: { race: LiveRaceView; date: string }) {
  return (
    <Link
      className="live-race-link"
      to={boardHref(race.race_id, date, { fromLive: true })}
      title="全頭盤・top5理由を見る"
    >
      <strong>{raceLabel(race)}</strong>
    </Link>
  );
}

// ソート可能な列見出し。クリックで「同列=方向トグル / 別列=その列の既定方向」。
// 状態列だけは正準の固定順（sortRaces が dir を反映しない）なので、方向表示を出さず
// aria-sort も "other" にする（▲/▼ と実際の並びの乖離を作らない）。
function SortTh({
  label,
  col,
  query,
  onSort,
}: {
  label: string;
  col: SortKey;
  query: LiveQuery;
  onSort: (key: SortKey) => void;
}) {
  const active = query.sort === col;
  const fixedOrder = col === "status";
  const ariaSort = !active
    ? undefined
    : fixedOrder
      ? "other"
      : query.dir === "asc"
        ? "ascending"
        : "descending";
  return (
    <th aria-sort={ariaSort}>
      <button
        type="button"
        className={`live-sort${active ? " live-sort-active" : ""}`}
        onClick={() => onSort(col)}
      >
        {label}
        {active && !fixedOrder && (query.dir === "asc" ? " ▲" : " ▼")}
      </button>
    </th>
  );
}

// 絞り込みチップ 1 個。タブと同じ見た目を流用する。
function FilterChip({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className={`tab${active ? " tab-active" : ""}`}
      aria-pressed={active}
      onClick={onClick}
    >
      {label}
    </button>
  );
}

// thead の <th> 本数（SortTh 6 + 単勝 + 注記）。列を増減したらここも更新する
// （SlipRow の colSpan がズレると伝票展開行のレイアウトが崩れる）。
const COLS = 8;

// 🟢買いレースの伝票（そのまま買える形: 式別 / 方式 / 軸 / 相手 / 点数 / 金額）。
// 買いにだけ出す（#344「在庫は常に出すが買いに見せない」）。
function SlipRow({ race }: { race: LiveRaceView }) {
  const groups = groupLegs(race.slip.legs);
  return (
    <tr className="live-slip-row">
      <td colSpan={COLS}>
        <p className="muted live-slip-head">
          ◎{maru(race.axis)} 複勝帯{" "}
          {placeBand(race.axis_place_odds_low, race.axis_place_odds_high)}
        </p>
        <table className="grid live-slip">
          <thead>
            <tr>
              <th>式別</th>
              <th>方式</th>
              <th>軸</th>
              <th>相手</th>
              <th>点数</th>
              <th>金額</th>
            </tr>
          </thead>
          <tbody>
            {groups.map((g) => (
              <tr key={`${g.betType}-${g.method}`}>
                <td>{BET_TYPE_JP[g.betType] ?? g.betType}</td>
                <td>
                  {METHOD_JP[g.method] ?? g.method}
                  {g.method === "box" && "（軸なし）"}
                </td>
                <td>{g.axis != null ? `◎${maru(g.axis)}` : "—"}</td>
                <td>{g.members.map(maru).join("")}</td>
                <td>{g.points}点</td>
                <td>{yen(g.amount)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </td>
    </tr>
  );
}

// テーブル 1 行（+ 買いなら伝票展開行）。行クリックで伝票トグル（リンクは除外）。
function LiveRow({
  race,
  date,
  rowClass,
  started,
  notes,
  chip,
  isBuy,
  slipOpen,
  onToggle,
}: {
  race: LiveRaceView;
  date: string;
  rowClass: string | undefined;
  started: boolean;
  notes: string[];
  chip: string | null;
  isBuy: boolean;
  slipOpen: boolean;
  onToggle: () => void;
}) {
  const flipped = notes.length > 0;
  return (
    <>
      <tr
        className={rowClass}
        onClick={(e) => {
          // レース名リンク等のクリックは行トグルにしない（買い行のみトグル対象）。
          if (!isBuy || (e.target as HTMLElement).closest("a")) return;
          onToggle();
        }}
      >
        <td className="live-status">
          {isBuy && (
            <button
              type="button"
              className="live-slip-toggle"
              aria-expanded={slipOpen}
              aria-label={slipOpen ? "伝票を折りたたむ" : "伝票を展開"}
              onClick={(e) => {
                // 行クリックのトグルと二重発火させない。
                e.stopPropagation();
                onToggle();
              }}
            >
              {slipOpen ? "▼" : "▶"}
            </button>
          )}
          {started && "⚫終 "}
          {tierShort(race.tier)}
          {flipped && "🔶"}
        </td>
        <td>
          <RaceLabelLink race={race} date={date} />
        </td>
        <td>{race.post_time ?? "—"}</td>
        <td className={isBuy ? "live-roi" : undefined}>{roiPct(race.roi)}</td>
        <td>
          ◎{maru(race.axis)} ({race.axis_prob.toFixed(0)}%)
        </td>
        <td>
          {race.axis_win_odds != null ? race.axis_win_odds.toFixed(1) : "—"}
        </td>
        <td>
          {chip && <span className="live-tag chip-rough">{chip}</span>}
          {race.konsen && <span className="live-tag">混戦</span>}
        </td>
        <td className="live-notes">
          {notes.map((n) => (
            <span key={n} className="live-flip">
              🔶 {n}
            </span>
          ))}
          {/* 断然人気の見送り理由は −EV 局面の注意喚起として残す（閾値は live.ts の共有定数）。 */}
          {!isBuy &&
            race.axis_win_odds != null &&
            race.axis_win_odds <= DANZEN_WIN_ODDS_MAX && (
              <span className="muted">
                {skipReason({
                  roi: race.roi,
                  axis: race.axis,
                  axis_win_odds: race.axis_win_odds,
                })}
              </span>
            )}
          {race.odds_missing && (
            <span className="muted">※ オッズ欠落・ROI 過小評価の可能性</span>
          )}
        </td>
      </tr>
      {slipOpen && <SlipRow race={race} />}
    </>
  );
}

export function LiveBets() {
  const { date = "" } = useParams();
  const [searchParams, setSearchParams] = useSearchParams();
  const query = parseLiveQuery(searchParams);
  // 発走済み判定・鮮度の相対時刻用の「現在」。30 秒刻みで更新（純粋関数側は now 引数のまま）。
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const id = setInterval(() => setNow(new Date()), 30_000);
    return () => clearInterval(id);
  }, []);
  // 買い行の伝票は既定展開。「ユーザーが畳んだもの」だけ持つ（refetch で状態が飛ばない）。
  // route param の date 変更では再マウントされないため、日付を跨いだ残留は明示的に消す。
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  useEffect(() => {
    setCollapsed(new Set());
  }, [date]);

  const live = useQuery({
    queryKey: ["live", date],
    enabled: !!date,
    retry: false,
    // スナップショットは predict-watch のスイープ（5 分間隔）でしか変わらないため、
    // 未発走レースが残る間だけ 1 分間隔で自動再取得して新スイープを拾う（#372）。
    refetchInterval: (q) => {
      const rs = q.state.data?.races;
      const t = new Date();
      // スイープ開始前（データ未取得・races 0 件）は判定材料が無いので、
      // 過去日でなければポーリングを続けて初回スナップショットを自動で拾う。
      if (!rs || rs.length === 0) {
        return raceStarted(date, "23:59", t) === true ? false : 60_000;
      }
      return hasUpcomingRaces(rs, date, t) ? 60_000 : false;
    },
    queryFn: async () => {
      const { data, error } = await api.GET("/api/live/{date}", {
        params: { path: { date } },
      });
      if (error) throw new Error("ライブ EV の取得に失敗しました");
      return data;
    },
  });

  const races = useMemo(() => live.data?.races ?? [], [live.data]);
  // 段階ボードは floor 未満（tier=hidden）を隠す（#344）。絞り込み・ソートの前に除外。
  const shown = useMemo(() => races.filter((r) => r.tier !== "hidden"), [races]);
  const ctx = useMemo(() => ({ date, now }), [date, now]);
  const visible = useMemo(
    () =>
      sortRaces(
        filterRaces(shown, { status: query.status, verdict: query.verdict }, ctx),
        query.sort,
        query.dir,
        ctx,
      ),
    [shown, query.status, query.verdict, query.sort, query.dir, ctx],
  );
  const hiddenCount = races.length - shown.length;
  // races が空（スイープ開始前）のときは refetchInterval と同じ基準（過去日か否か）で
  // 判定し、「ポーリング継続中なのにバッジは監視終了」の不整合を作らない。
  const hasUpcoming =
    races.length === 0
      ? raceStarted(date, "23:59", now) !== true
      : hasUpcomingRaces(races, date, now);
  const fresh = live.data
    ? freshness(live.data.summary.last_updated, hasUpcoming, now)
    : null;

  // ルート live/:date により date は通常必ず存在するが、空だと query が無効化され
  // isPending のまま「読み込み中…」で固まるため、派生計算の前に明示的にガードする。
  if (!date) {
    return <p className="error">開催日が指定されていません。</p>;
  }

  // ソート・絞り込み状態は URL クエリに反映（リロード・共有耐性、#370）。既定値は省略。
  const applyQuery = (next: LiveQuery) =>
    setSearchParams(liveQueryParams(next), { replace: true });
  const onSort = (key: SortKey) =>
    applyQuery(
      // 状態列は固定順（sortRaces が dir 非対応）なのでトグルせず既定に戻すだけ。
      key === "status"
        ? { ...query, sort: "status", dir: "asc" }
        : query.sort === key
          ? { ...query, dir: query.dir === "asc" ? "desc" : "asc" }
          : { ...query, sort: key, dir: defaultDir(key) },
    );
  const setStatus = (status: StatusFilter) => applyQuery({ ...query, status });
  const setVerdict = (verdict: VerdictFilter) =>
    applyQuery({ ...query, verdict });
  const toggleSlip = (raceId: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(raceId)) next.delete(raceId);
      else next.add(raceId);
      return next;
    });

  return (
    <section>
      <div className="toolbar">
        <h2>ライブ EV {date}</h2>
        <Link to={`/?date=${date}`}>← レース一覧へ</Link>
        {/* API 再取得のみ（スナップショット自体は predict-watch が更新する）ため「再読込」。#372 */}
        <button onClick={() => live.refetch()} disabled={live.isFetching}>
          {live.isFetching ? "再読込中…" : "再読込"}
        </button>
        {live.data && fresh && (
          <span className="muted">
            最終更新 {jstHm(live.data.summary.last_updated)}
            {/* last_updated が無いとき「—（—）」の二重ダッシュにしない */}
            {fresh.label !== "—" && `（${fresh.label}）`}
          </span>
        )}
        {fresh?.state === "stale" && (
          <span className="live-stale">
            ⚠ {STALE_MINUTES}分以上スナップショット更新なし — predict-watch
            の稼働を確認
          </span>
        )}
        {fresh?.state === "done" && (
          <span className="badge">監視終了（全レース発走済み）</span>
        )}
      </div>

      {live.isPending && <p>読み込み中…</p>}
      {live.isError && !live.data && (
        <p className="error">{(live.error as Error).message}</p>
      )}
      {/* 再読込失敗時は前回データを残したまま警告だけ出す（全置換すると判断材料が消える）。 */}
      {live.isError && live.data != null && (
        <p className="live-stale">再読込に失敗しました（表示は前回取得のまま）</p>
      )}

      {live.data && (
        <>
          <p className="live-summary">{summaryLine(live.data.summary)}</p>

          {/* 表示対象が無いときは操作しても何も起きないため、チップ自体を出さない。 */}
          {shown.length > 0 && (
          <div className="live-filter">
            <span className="muted">状態:</span>
            <FilterChip
              label="全部"
              active={query.status === "all"}
              onClick={() => setStatus("all")}
            />
            <FilterChip
              label="未発走"
              active={query.status === "upcoming"}
              onClick={() => setStatus("upcoming")}
            />
            <FilterChip
              label="終了"
              active={query.status === "finished"}
              onClick={() => setStatus("finished")}
            />
            <span className="muted">判定:</span>
            <FilterChip
              label="全部"
              active={query.verdict === "all"}
              onClick={() => setVerdict("all")}
            />
            <FilterChip
              label="張り"
              active={query.verdict === "bet"}
              onClick={() => setVerdict("bet")}
            />
            <FilterChip
              label="見送り"
              active={query.verdict === "skip"}
              onClick={() => setVerdict("skip")}
            />
          </div>
          )}

          {races.length === 0 ? (
            <p className="muted">監視データなし</p>
          ) : shown.length === 0 ? (
            // 全レースが floor 未満のケース。絞り込みが原因でないことを誤案内しない。
            <p className="muted">
              全 {races.length} レースが当日 ROI 分布の下位（floor
              未満）で非表示。
            </p>
          ) : (
            <>
              {visible.length === 0 ? (
                <p className="muted">絞り込み条件に該当するレースなし</p>
              ) : (
              <table className="grid live-board">
                <thead>
                  <tr>
                    <SortTh label="状態" col="status" query={query} onSort={onSort} />
                    <SortTh label="レース" col="race" query={query} onSort={onSort} />
                    <SortTh label="発走" col="post" query={query} onSort={onSort} />
                    <SortTh label="ROI" col="roi" query={query} onSort={onSort} />
                    <SortTh label="軸" col="axisProb" query={query} onSort={onSort} />
                    <th>単勝</th>
                    <SortTh label="荒れ" col="rough" query={query} onSort={onSort} />
                    <th>注記</th>
                  </tr>
                </thead>
                <tbody>
                  {visible.map((r) => {
                    const started =
                      raceStarted(date, r.post_time, now) === true;
                    const soon = !started && isSoon(date, r.post_time, now);
                    const notes = flipNotes(r.flip, {
                      axis: r.axis,
                      roi: r.roi,
                      verdict: r.verdict,
                    });
                    const isBuy = r.tier === "buy";
                    const rowClass =
                      [
                        started ? "live-row-done" : "",
                        soon ? "live-row-soon" : "",
                        isBuy ? "live-row-buy" : "",
                      ]
                        .filter(Boolean)
                        .join(" ") || undefined;
                    return (
                      <LiveRow
                        key={r.race_id}
                        race={r}
                        date={date}
                        rowClass={rowClass}
                        started={started}
                        notes={notes}
                        chip={roughnessChip(r.roughness, r.roughness_label)}
                        isBuy={isBuy}
                        slipOpen={isBuy && !collapsed.has(r.race_id)}
                        onToggle={() => toggleSlip(r.race_id)}
                      />
                    );
                  })}
                </tbody>
              </table>
              )}
              {hiddenCount > 0 && (
                <p className="muted">
                  他 {hiddenCount} レースは当日 ROI 分布の下位（floor
                  未満）で非表示。
                </p>
              )}
            </>
          )}
        </>
      )}
    </section>
  );
}
