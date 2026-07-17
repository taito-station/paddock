import { useEffect, useState } from "react";
import { useParams, Link } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import { recoveryRate, yen } from "../lib/format";
import { raceListHref } from "../lib/header-date";
import { hasUnsettledRaces } from "../lib/dashboard";
import { useResultsRefresh } from "../lib/useResultsRefresh";

export function SessionSummary() {
  const { date = "" } = useParams();
  const qc = useQueryClient();
  const [budget, setBudget] = useState("10000");
  // 発走済み判定・ポーリング gate 用の現在時刻（30 秒 tick）。
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const id = setInterval(() => setNow(new Date()), 30_000);
    return () => clearInterval(id);
  }, []);

  const session = useQuery({
    queryKey: ["session", date],
    enabled: !!date,
    retry: false,
    queryFn: async () => {
      const { data, error, response } = await api.GET("/api/sessions/{date}", {
        params: { path: { date } },
      });
      if (response.status === 404) return null;
      if (error) throw new Error("セッションの取得に失敗しました");
      return data;
    },
  });

  // 自動精算ポーリングの gate に使うレース一覧（post_time・result_confirmed）。
  const races = useQuery({
    queryKey: ["races", date],
    enabled: !!date,
    queryFn: async () => {
      const { data, error } = await api.GET("/api/races", {
        params: { query: { date } },
      });
      if (error) throw new Error("レース一覧の取得に失敗しました");
      return data;
    },
  });

  const create = useMutation({
    mutationFn: async (budgetYen: number) => {
      const { data, error } = await api.POST("/api/sessions/{date}", {
        params: { path: { date } },
        body: { budget: budgetYen },
      });
      if (error) throw new Error("セッションの作成に失敗しました");
      return data;
    },
    onSuccess: (data) => qc.setQueryData(["session", date], data),
  });

  const settle = useMutation({
    mutationFn: async () => {
      const { data, error } = await api.POST(
        "/api/sessions/{date}/results:refresh",
        { params: { path: { date } } },
      );
      if (error) throw new Error("確定結果の取得に失敗しました");
      return data;
    },
    // 精算で残高・払戻が変わるためセッションを再取得する。
    onSuccess: () => qc.invalidateQueries({ queryKey: ["session", date] }),
  });

  // 結果確定を検知して自動精算する（#381）。ライブ一覧と同じ gate（発走済み・未確定が残る間だけ）に
  // 揃え、発走前の空回りを避ける（過去日・全確定で停止）。手動「精算」ボタンはフォールバックとして残す。
  useResultsRefresh(date, {
    enabled: hasUnsettledRaces(races.data?.races ?? [], date, now),
    now,
  });

  return (
    <section>
      <div className="toolbar">
        <h2>収支 {date}</h2>
        <Link to={raceListHref(date)}>← レース一覧へ</Link>
      </div>

      {session.isPending && <p>読み込み中…</p>}
      {session.isError && (
        <p className="error">{(session.error as Error).message}</p>
      )}

      {session.isSuccess && session.data === null && (
        <form
          className="toolbar"
          onSubmit={(e) => {
            e.preventDefault();
            const n = Number(budget);
            // 予算は 1000 円単位（input の step と一致）。下限・有限性を確認して送る。
            if (Number.isFinite(n) && n >= 1000) create.mutate(n);
          }}
        >
          <span className="muted">この開催日のセッションは未作成です。</span>
          <label>
            予算{" "}
            <input
              type="number"
              // min を step の倍数に揃える（min=1 だと 1,1001,… のグリッドになり 1 万等が
              // HTML5 検証で弾かれるため）。予算は 1000 円単位とする。
              min={1000}
              step={1000}
              value={budget}
              onChange={(e) => setBudget(e.target.value)}
            />
            円
          </label>
          <button
            type="submit"
            disabled={
              create.isPending ||
              !Number.isFinite(Number(budget)) ||
              Number(budget) < 1000
            }
          >
            セッション開始
          </button>
          {create.isError && (
            <span className="error">{(create.error as Error).message}</span>
          )}
        </form>
      )}

      {session.data && (
        <>
          <table className="grid kv">
            <tbody>
              <tr>
                <th>状態</th>
                <td>{session.data.completed ? "完了" : "進行中"}</td>
              </tr>
              <tr>
                <th>開始予算</th>
                <td>{yen(session.data.budget)}</td>
              </tr>
              <tr>
                <th>残高</th>
                <td>{yen(session.data.balance)}</td>
              </tr>
              <tr>
                <th>総賭け金</th>
                <td>{yen(session.data.total_bet)}</td>
              </tr>
              <tr>
                <th>総払戻</th>
                <td>{yen(session.data.total_payout)}</td>
              </tr>
              <tr>
                <th>損益</th>
                <td className={session.data.pnl < 0 ? "error" : ""}>
                  {session.data.pnl >= 0 ? "+" : ""}
                  {yen(session.data.pnl)}
                </td>
              </tr>
              <tr>
                <th>回収率</th>
                <td>
                  {(() => {
                    const r = recoveryRate(
                      session.data.total_payout,
                      session.data.total_bet,
                    );
                    return r === null ? "-" : `${r.toFixed(1)}%`;
                  })()}
                </td>
              </tr>
            </tbody>
          </table>

          <div className="toolbar" style={{ marginTop: "1rem" }}>
            <button onClick={() => settle.mutate()} disabled={settle.isPending}>
              確定結果で精算（最新取得）
            </button>
            {settle.isError && (
              <span className="error">{(settle.error as Error).message}</span>
            )}
            {settle.isSuccess && (
              <span className="muted">
                精算: {settle.data.settled_races}R 確定 / 残高{" "}
                {yen(settle.data.balance)}
              </span>
            )}
          </div>

          <h3 style={{ marginTop: "1.5rem" }}>買い目明細</h3>
          {session.data.bets.length === 0 ? (
            <p className="muted">まだ買い目がありません。</p>
          ) : (
            <table className="grid">
              <thead>
                <tr>
                  <th>レース</th>
                  <th>券種</th>
                  <th>組合せ</th>
                  <th>賭け金</th>
                  <th>払戻</th>
                  <th>EV</th>
                </tr>
              </thead>
              <tbody>
                {session.data.bets.map((b, i) => (
                  <tr key={`${b.race_id}-${b.bet_type}-${b.combination}-${i}`}>
                    <td>{b.race_id}</td>
                    <td>{b.bet_type}</td>
                    <td>{b.combination}</td>
                    <td>{yen(b.stake)}</td>
                    <td>{yen(b.payout)}</td>
                    <td>{b.ev.toFixed(2)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </>
      )}
    </section>
  );
}
