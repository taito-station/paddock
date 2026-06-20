import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import { SURFACE_JP, VENUE_JP, todayJst } from "../lib/format";

export function RaceList() {
  const [date, setDate] = useState(todayJst);

  const races = useQuery({
    queryKey: ["races", date],
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
        {session.data ? (
          <span className="session-balance">
            残高 {session.data.balance.toLocaleString()}円 / 予算{" "}
            {session.data.budget.toLocaleString()}円
            {session.data.completed ? "（完了）" : "（進行中）"}
          </span>
        ) : (
          <span className="muted">セッション未作成</span>
        )}
      </div>

      {races.isPending && <p>読み込み中…</p>}
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
                <td>{r.race_num}</td>
                <td>{VENUE_JP[r.venue] ?? r.venue}</td>
                <td>{r.distance}m</td>
                <td>{SURFACE_JP[r.surface] ?? r.surface}</td>
                <td>
                  {boughtRaceIds.has(r.race_id) ? (
                    <span className="badge badge-bought">購入済み</span>
                  ) : sessionCompleted ? (
                    // 完了セッションで未購入＝買わなかった（見送り）。
                    <span className="badge">見送り</span>
                  ) : hasSession ? (
                    <span className="badge">未処理</span>
                  ) : (
                    // セッション未作成時は購入状況が不明なのでバッジを出さない。
                    <span className="muted">-</span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
