import { useEffect, useState } from "react";
import { useParams, Link } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import { recoveryRate, yen } from "../lib/format";
import { raceListHref } from "../lib/header-date";
import { hasUnsettledRaces } from "../lib/dashboard";
import { useResultsRefresh } from "../lib/useResultsRefresh";
import { useSessionQuery, useRacesQuery } from "../lib/queries";
import { isUnitOf } from "../lib/bets";
import {
  CLOCK_TICK_INTERVAL_MS,
  DEFAULT_SESSION_BUDGET,
} from "../lib/constants";

export function SessionSummary() {
  const { date = "" } = useParams();
  const qc = useQueryClient();
  const [budget, setBudget] = useState(DEFAULT_SESSION_BUDGET);
  // 発走済み判定・ポーリング gate 用の現在時刻（30 秒 tick）。
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const id = setInterval(() => setNow(new Date()), CLOCK_TICK_INTERVAL_MS);
    return () => clearInterval(id);
  }, []);

  const session = useSessionQuery(date);

  // 自動精算ポーリングの gate に使うレース一覧（post_time・result_confirmed）。
  const races = useRacesQuery(date);

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
    // 精算で残高・払戻が変わるためセッションを再取得する。着順取り込みで races/live の
    // result_confirmed も変わるため併せて無効化し、ポーリング gate（hasUnsettledRaces）と
    // 一覧表示を即時同期する（手動精算後にポーリングが止まらない stale gate を防ぐ）。
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["session", date] });
      void qc.invalidateQueries({ queryKey: ["races", date] });
      void qc.invalidateQueries({ queryKey: ["live", date] });
    },
  });

  // 結果確定を検知して自動精算する（#381）。ライブ一覧と同じ gate（発走済み・未確定が残る間だけ）に
  // 揃え、発走前の空回りを避ける（過去日・全確定で停止）。手動「精算」ボタンはフォールバックとして残す。
  const settlePoll = useResultsRefresh(date, {
    enabled: hasUnsettledRaces(races.data?.races ?? [], date, now),
    now,
  });

  // セッション総予算は 1000 円単位（input の step と一致）。有限・下限・単位を満たすときだけ
  // 作成できる。submit と disabled の両方でこの判定を使う（HTML の step はヒントに過ぎず、
  // 1500 等の非 1000 倍数が payload に素通りするのを防ぐ。#412 と方針を揃える・#424）。
  const budgetNum = Number(budget);
  // 下限（1000 円以上）を満たすか。NaN・非有限は比較が false になり弾かれる。
  const budgetInRange = budgetNum >= 1000;
  const budgetValid = budgetInRange && isUnitOf(budgetNum, 1000);
  // 下限は満たすが 1000 円単位でない端数（1500 等）は黙って丸めず明示エラーを出し、入力は残す。
  const budgetUnitError = budgetInRange && !isUnitOf(budgetNum, 1000);

  return (
    <section>
      <div className="toolbar">
        <h2>収支 {date}</h2>
        <Link to={raceListHref(date)}>← レース一覧へ</Link>
      </div>

      {session.isPending && <p>読み込み中…</p>}
      {session.isError && (
        <p className="error">{session.error.message}</p>
      )}

      {session.isSuccess && session.data === null && (
        <form
          className="toolbar"
          onSubmit={(e) => {
            e.preventDefault();
            // 有限・下限・1000 円単位を満たすときだけ送る（防御として disabled と二重化）。
            if (budgetValid) create.mutate(budgetNum);
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
          <button type="submit" disabled={create.isPending || !budgetValid}>
            セッション開始
          </button>
          {budgetUnitError && (
            <span className="error">予算は 1000 円単位で入力してください</span>
          )}
          {create.isError && (
            <span className="error">{create.error.message}</span>
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

          {/* 自動精算ポーリングの失敗を明示（#478）。無言停止を避け、直下の手動精算ボタンへ誘導する。 */}
          {settlePoll.isError && (
            <p className="live-stale mt-lg">
              ⚠ 自動精算に失敗しています。下の「確定結果で精算」で手動再試行できます。
            </p>
          )}

          <div className="toolbar mt-lg">
            <button onClick={() => settle.mutate()} disabled={settle.isPending}>
              確定結果で精算（最新取得）
            </button>
            {settle.isError && (
              <span className="error">{settle.error.message}</span>
            )}
            {settle.isSuccess && (
              <span className="muted">
                精算: {settle.data.settled_races}R 確定 / 残高{" "}
                {yen(settle.data.balance)}
              </span>
            )}
          </div>

          <h3 className="mt-2xl">買い目明細</h3>
          {session.data.bets.length === 0 ? (
            <p className="muted">まだ買い目がありません。</p>
          ) : (
            <div
              className="table-scroll"
              role="region"
              aria-label="買い目明細（横スクロール可）"
              tabIndex={0}
            >
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
            </div>
          )}
        </>
      )}
    </section>
  );
}
