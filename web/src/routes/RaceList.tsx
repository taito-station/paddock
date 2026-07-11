import { useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import {
  SURFACE_JP,
  VENUE_JP,
  raceBadge,
  todayJst,
  type RaceBadge,
} from "../lib/format";
import { boardHref } from "../lib/live";

function Badge({ kind }: { kind: RaceBadge }) {
  switch (kind) {
    case "bought":
      return <span className="badge badge-bought">購入済み</span>;
    case "skipped":
      return <span className="badge">見送り</span>;
    case "pending":
      return <span className="badge">未処理</span>;
    case "none":
      // セッション未作成時は購入状況が不明なのでバッジを出さない。
      return <span className="muted">-</span>;
  }
}

export function RaceList() {
  // 収支ビュー等からの ?date= 深リンクを初期値に使う（無ければ JST の今日）。
  const [searchParams] = useSearchParams();
  const [date, setDate] = useState(() => searchParams.get("date") || todayJst());

  const races = useQuery({
    queryKey: ["races", date],
    // 日付クリア時（空文字）は叩かない（API 400 のエラー点滅を防ぐ）。
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

  const hasSession = !!session.data;
  const sessionCompleted = session.data?.completed ?? false;
  const boughtRaceIds = useMemo(
    () => new Set(session.data?.bets.map((b) => b.race_id) ?? []),
    [session.data],
  );

  return (
    <section>
      <div className="toolbar">
        <label>
          開催日{" "}
          <input
            type="date"
            value={date}
            onChange={(e) => setDate(e.target.value)}
          />
        </label>
        {session.isError ? (
          // 404 は queryFn が null に倒す。ここに来るのは 500・ネットワーク断などの
          // 実障害なので「未作成」と取り違えず失敗を明示する。
          <span className="error">残高の取得に失敗しました</span>
        ) : session.data ? (
          <span className="session-balance">
            残高 {session.data.balance.toLocaleString()}円 / 予算{" "}
            {session.data.budget.toLocaleString()}円
            {session.data.completed ? "（完了）" : "（進行中）"}
          </span>
        ) : (
          <span className="muted">セッション未作成</span>
        )}
        {date && <Link to={`/sessions/${date}`}>収支</Link>}
      </div>

      {!date && <p className="muted">開催日を選択してください。</p>}
      {date && races.isPending && <p>読み込み中…</p>}
      {races.isError && <p className="error">{races.error.message}</p>}
      {races.data && races.data.races.length === 0 && (
        <p className="muted">この開催日のレースはありません。</p>
      )}

      {races.data && races.data.races.length > 0 && (
        <table className="grid">
          <thead>
            <tr>
              <th>R</th>
              <th>開催</th>
              <th>距離</th>
              <th>馬場</th>
              <th>状態</th>
            </tr>
          </thead>
          <tbody>
            {races.data.races.map((r) => (
              <tr key={r.race_id}>
                <td>
                  {/* 盤＝唯一のレースビュー（#377 で RaceDetail を統合・廃止） */}
                  <Link to={boardHref(r.race_id, date)}>{r.race_num}</Link>
                </td>
                <td>{VENUE_JP[r.venue] ?? r.venue}</td>
                <td>{r.distance}m</td>
                <td>{SURFACE_JP[r.surface] ?? r.surface}</td>
                <td>
                  <Badge
                    kind={raceBadge({
                      bought: boughtRaceIds.has(r.race_id),
                      hasSession,
                      completed: sessionCompleted,
                    })}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
