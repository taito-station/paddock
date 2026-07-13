import { useEffect, useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import {
  SURFACE_JP,
  VENUE_JP,
  raceBadge,
  todayJst,
  yen,
  type RaceBadge,
} from "../lib/format";
import {
  dashboardQueryParams,
  filterRows,
  joinRaces,
  sortRows,
  type DashboardRow,
} from "../lib/dashboard";
import {
  STALE_MINUTES,
  backParam,
  boardHref,
  defaultDir,
  freshness,
  hasUpcomingRaces,
  isPastDate,
  jstHm,
  parseLiveQuery,
  raceStarted,
  summaryLine,
  type LiveQuery,
  type SortKey,
  type StatusFilter,
  type VerdictFilter,
} from "../lib/live";
import { SortTh } from "./racelist/SortTh";
import { FilterChip } from "./racelist/FilterChip";
import { Badge } from "./racelist/Badge";
import { DashboardRowView } from "./racelist/DashboardRowView";

// 縮退モードの 1 行（旧 RaceList 相当。snapshot の無い日はこの静的一覧を出す）。
function StaticRow({
  row,
  date,
  back,
  badge,
}: {
  row: DashboardRow;
  date: string;
  back: string;
  badge: RaceBadge;
}) {
  const r = row.race;
  return (
    <tr>
      <td>
        <Link to={boardHref(r.race_id, date, back)}>{r.race_num}</Link>
      </td>
      <td>{VENUE_JP[r.venue] ?? r.venue}</td>
      {/* 発走時刻は race_cards 正本（#391）。未取得は "—"（不明扱い）。 */}
      <td>{r.post_time ?? "—"}</td>
      <td>{r.distance}m</td>
      <td>{SURFACE_JP[r.surface] ?? r.surface}</td>
      <td>
        <Badge kind={badge} />
      </td>
    </tr>
  );
}

// レース一覧＝日次ダッシュボード（#378 で /live を統合）。
// /api/races（DB 正本）× /api/live/{date}（EV snapshot）× /api/sessions/{date}（購入状態）を
// race_id で突合し、snapshot の無い日は静的一覧に縮退する。
export function RaceList() {
  const [searchParams, setSearchParams] = useSearchParams();
  // 開催日・ソート・絞り込みは URL クエリが正（リロード・共有耐性）。date 省略時は JST の今日。
  const date = searchParams.get("date") || todayJst();
  const query = parseLiveQuery(searchParams);
  // 盤へ引き継ぐ絞り込み状態（#380）。盤の「← レース一覧」で復元する。date は盤 URL の
  // ?date= が別途持つため back には含めない（backParam は sort/filter のみ直列化）。
  const back = backParam(query);

  // 発走済み判定・鮮度の相対時刻用の「現在」。30 秒刻みで更新（純粋関数側は now 引数のまま）。
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const id = setInterval(() => setNow(new Date()), 30_000);
    return () => clearInterval(id);
  }, []);
  // 買い行の伝票は既定展開。「ユーザーが畳んだもの」だけ持つ（refetch で状態が飛ばない）。
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  useEffect(() => {
    setCollapsed(new Set());
  }, [date]);

  const races = useQuery({
    queryKey: ["races", date],
    // date は todayJst フォールバックで常に非空だが、queryKey の前提を守る防御として残す。
    enabled: !!date,
    queryFn: async () => {
      const { data, error } = await api.GET("/api/races", {
        params: { query: { date } },
      });
      if (error) throw new Error("レース一覧の取得に失敗しました");
      return data;
    },
  });

  // セッションは未作成だと 404。404 のみ「残高表示なし」に倒し、それ以外の障害は
  // 握り潰さず投げる（500・ネットワーク断を「未作成」と取り違えないため）。
  const session = useQuery({
    queryKey: ["session", date],
    enabled: !!date,
    retry: false,
    queryFn: async () => {
      const { data, error, response } = await api.GET("/api/sessions/{date}", {
        params: { path: { date } },
      });
      if (response.status === 404) return null;
      if (error) throw new Error("セッション収支の取得に失敗しました");
      return data;
    },
  });

  const live = useQuery({
    queryKey: ["live", date],
    enabled: !!date,
    retry: false,
    // スナップショットは predict-watch のスイープ（5 分間隔）でしか変わらないため、
    // 未発走レースが残る間だけ 1 分間隔で自動再取得して新スイープを拾う（#372）。
    refetchInterval: (q) => {
      const rs = q.state.data?.races;
      const t = new Date();
      // 非開催日（DB レース 0 件が確定）と races 取得エラー時は snapshot を追っても
      // 得るものが無いので止める（トップページ常駐でも無駄打ちしない）。
      // races ロード中は判定を保留して継続。
      if (races.isError) return false;
      if (races.data != null && races.data.races.length === 0) return false;
      // スイープ開始前（snapshot 未取得・races 0 件）は判定材料が無いので、
      // 過去日でなければポーリングを続けて初回スナップショットを自動で拾う。
      if (!rs || rs.length === 0) {
        return isPastDate(date, t) ? false : 60_000;
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

  const hasSession = !!session.data;
  const sessionCompleted = session.data?.completed ?? false;
  const boughtRaceIds = useMemo(
    () => new Set(session.data?.bets.map((b) => b.race_id) ?? []),
    [session.data],
  );

  const liveRaces = live.data?.races;
  // snapshot が 1 件でもあればライブモード（統合テーブル）。無い日は静的一覧に縮退。
  const liveMode = !!liveRaces && liveRaces.length > 0;
  const ctx = useMemo(() => ({ date, now }), [date, now]);
  const rows = useMemo(
    () => joinRaces(races.data?.races ?? [], liveRaces, boughtRaceIds),
    [races.data, liveRaces, boughtRaceIds],
  );
  const visible = useMemo(
    () =>
      sortRows(
        filterRows(rows, { status: query.status, verdict: query.verdict }, ctx),
        query.sort,
        query.dir,
        ctx,
      ),
    [rows, query.status, query.verdict, query.sort, query.dir, ctx],
  );

  // 鮮度（#372）。snapshot が空（スイープ開始前）のときは refetchInterval と同じ基準
  //（過去日か否か）で判定し、「ポーリング継続中なのにバッジは監視終了」の不整合を作らない。
  const snapshotRaces = liveRaces ?? [];
  const hasUpcoming =
    snapshotRaces.length === 0
      ? !isPastDate(date, now)
      : hasUpcomingRaces(snapshotRaces, date, now);
  const fresh = live.data
    ? freshness(live.data.summary.last_updated, hasUpcoming, now)
    : null;

  // ソート・絞り込み・日付は URL クエリに反映（既定値は省略）。replace は意図的:
  // チップ連打で履歴を汚さない。date の併存は dashboardQueryParams がマージする。
  // 日付切替はヘッダ常駐ピッカーへ一本化済み（#379）。ここは sort/filter のみ更新し、
  // date は現在値を dashboardQueryParams でマージ保持する。
  const applyQuery = (next: LiveQuery) =>
    setSearchParams(dashboardQueryParams(next, date), { replace: true });
  const onSort = (key: SortKey) =>
    applyQuery(
      // 状態列は固定順（sortRows が dir 非対応）なのでトグルせず既定に戻すだけ。
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

  const badgeOf = (bought: boolean) =>
    raceBadge({ bought, hasSession, completed: sessionCompleted });

  return (
    <section>
      <div className="toolbar">
        {/* 開催日の切替はヘッダ常駐ピッカーに一本化（#379）。ここでは残高等のみ表示。 */}
        {session.isError ? (
          // 404 は queryFn が null に倒す。ここに来るのは 500・ネットワーク断などの
          // 実障害なので「未作成」と取り違えず失敗を明示する。
          <span className="error">残高の取得に失敗しました</span>
        ) : session.data ? (
          <span className="session-balance">
            残高 {yen(session.data.balance)} / 予算 {yen(session.data.budget)}
            {session.data.completed ? "（完了）" : "（進行中）"}
          </span>
        ) : (
          <span className="muted">セッション未作成</span>
        )}
        {/* date は ?date= 由来のユーザ制御値になりうるため必ずエンコード（boardHref と同基準） */}
        <Link to={`/sessions/${encodeURIComponent(date)}`}>収支</Link>
        {liveMode && (
          <>
            {/* API 再取得のみ（スナップショット自体は predict-watch が更新する）ため「再読込」。 */}
            <button onClick={() => live.refetch()} disabled={live.isFetching}>
              {live.isFetching ? "再読込中…" : "再読込"}
            </button>
            {fresh && (
              <span className="muted">
                最終更新 {jstHm(live.data!.summary.last_updated)}
                {/* last_updated が無いとき「—（—）」の二重ダッシュにしない */}
                {fresh.label !== "—" && `（${fresh.label}）`}
              </span>
            )}
          </>
        )}
        {/* 開催前日など JST でその日が始まる前、および非開催日（DB レース 0 件）は
            predict-watch 停止が正常なので警告しない（既定トップページの誤警報防止）。 */}
        {fresh?.state === "stale" &&
          raceStarted(date, "0:00", now) === true &&
          (races.data?.races.length ?? 0) > 0 && (
            <span className="live-stale">
              {/* label "—" = 更新時刻が読めていない（null/不正。経過時間の主張はできない） */}
              {fresh.label === "—"
                ? "⚠ スナップショット未取得 — predict-watch の稼働を確認"
                : `⚠ ${STALE_MINUTES}分以上スナップショット更新なし — predict-watch の稼働を確認`}
            </span>
          )}
        {fresh?.state === "done" && liveMode && (
          <span className="badge">監視終了（全レース発走済み）</span>
        )}
      </div>

      {/* ライブ EV の取得失敗はページを壊さず注記に縮退。前回データを保持したまま
          再取得に失敗した場合は「前回のまま」と明示する（鮮度の誤認防止）。 */}
      {live.isError && (
        <p className="live-stale">
          {liveMode
            ? "ライブ EV の再取得に失敗しました（表示は前回取得のまま）"
            : "ライブ EV の取得に失敗 — 一覧のみ表示"}
        </p>
      )}
      {liveMode && live.data && (
        <p className="live-summary">{summaryLine(live.data.summary)}</p>
      )}

      {races.isError ? (
        // races の確定エラーは live の決着を待たず即表示する。
        <p className="error">{(races.error as Error).message}</p>
      ) : races.isPending || live.isPending ? (
        // 列構成が静的 → ライブへジャンプするチラつきを防ぐため、両クエリの決着を待つ
        //（v5 の status は排他なので isPending 中に isError は立たない）。
        <p>読み込み中…</p>
      ) : rows.length === 0 ? (
        <p className="muted">この開催日のレースはありません。</p>
      ) : !liveMode ? (
        // ---- 縮退: snapshot の無い日は現行相当の静的一覧 ----
        <table className="grid">
          <thead>
            <tr>
              <th>R</th>
              <th>開催</th>
              <th>発走</th>
              <th>距離</th>
              <th>馬場</th>
              <th>状態</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <StaticRow
                key={row.race.race_id}
                row={row}
                date={date}
                back={back}
                badge={badgeOf(row.bought)}
              />
            ))}
          </tbody>
        </table>
      ) : (
        // ---- ライブモード: 統合ダッシュボード ----
        <>
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

          {visible.length === 0 ? (
            <p className="muted">絞り込み条件に該当するレースなし</p>
          ) : (
            // 列を増減したら racelist/SlipRow.tsx の DASHBOARD_COLS も更新すること。
            <table className="grid live-board">
              <thead>
                <tr>
                  <SortTh label="状態" col="status" query={query} onSort={onSort} />
                  <SortTh label="レース" col="race" query={query} onSort={onSort} />
                  <SortTh label="発走" col="post" query={query} onSort={onSort} />
                  <th>距離</th>
                  <SortTh label="ROI" col="roi" query={query} onSort={onSort} />
                  <SortTh label="軸" col="axisProb" query={query} onSort={onSort} />
                  <SortTh label="荒れ" col="rough" query={query} onSort={onSort} />
                  <th>購入</th>
                  <th>注記</th>
                </tr>
              </thead>
              <tbody>
                {visible.map((row) => (
                  <DashboardRowView
                    key={row.race.race_id}
                    row={row}
                    date={date}
                    back={back}
                    now={now}
                    badge={badgeOf(row.bought)}
                    slipOpen={
                      row.live?.tier === "buy" &&
                      !collapsed.has(row.race.race_id)
                    }
                    onToggle={() => toggleSlip(row.race.race_id)}
                  />
                ))}
              </tbody>
            </table>
          )}
        </>
      )}
    </section>
  );
}
