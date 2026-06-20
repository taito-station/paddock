import { useMemo, useState } from "react";
import { useParams, useNavigate, Link } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, type BetInput, type RecommendationBet } from "../api/client";
import { pct, yen, VENUE_JP } from "../lib/format";

const DEFAULT_RACE_BUDGET = 5000; // CLI predict の既定 race_budget と揃える。

type Edit = { stake: number; payout: number };
const betKey = (b: RecommendationBet) => `${b.bet_type}-${b.combination}`;

export function RaceDetail() {
  const { date = "", raceId = "" } = useParams();
  const qc = useQueryClient();
  const navigate = useNavigate();

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

  const balance = session.data?.balance ?? 0;
  const [budget, setBudget] = useState<number>(DEFAULT_RACE_BUDGET);
  const [edits, setEdits] = useState<Record<string, Edit>>({});

  const card = useQuery({
    queryKey: ["card", raceId],
    enabled: !!raceId,
    queryFn: async () => {
      const { data, error } = await api.GET("/api/races/{race_id}", {
        params: { path: { race_id: raceId } },
      });
      if (error) throw new Error("出馬表の取得に失敗しました");
      return data;
    },
  });

  const prediction = useQuery({
    queryKey: ["prediction", raceId],
    enabled: !!raceId,
    queryFn: async () => {
      const { data, error } = await api.GET(
        "/api/races/{race_id}/prediction",
        { params: { path: { race_id: raceId } } },
      );
      if (error) throw new Error("確率推定の取得に失敗しました");
      return data;
    },
  });

  const cap = Math.min(budget, balance);
  const recs = useQuery({
    queryKey: ["recommendations", raceId, cap],
    enabled: !!raceId && cap > 0,
    queryFn: async () => {
      const { data, error } = await api.GET(
        "/api/races/{race_id}/recommendations",
        { params: { path: { race_id: raceId }, query: { budget: cap } } },
      );
      if (error) throw new Error("買い目推奨の取得に失敗しました");
      return data;
    },
  });

  const oddsRefresh = useMutation({
    mutationFn: async () => {
      const { data, error } = await api.POST(
        "/api/sessions/{date}/races/{race_id}/odds:refresh",
        { params: { path: { date, race_id: raceId } } },
      );
      if (error) throw new Error("オッズの取得に失敗しました");
      return data;
    },
    onSuccess: () =>
      qc.invalidateQueries({ queryKey: ["recommendations", raceId] }),
  });

  const record = useMutation({
    mutationFn: async (bets: BetInput[]) => {
      const { data, error } = await api.POST(
        "/api/sessions/{date}/races/{race_id}/outcome",
        { params: { path: { date, race_id: raceId } }, body: { bets } },
      );
      if (error) throw new Error("記録に失敗しました（残高超過・二重記録の可能性）");
      return data;
    },
    onSuccess: (data) => {
      // サーバ確定値でセッションを更新し、収支ビューへ遷移する。
      qc.setQueryData(["session", date], data);
      navigate(`/sessions/${date}`);
    },
  });

  const bets = recs.data?.bets ?? [];
  const effStake = (b: RecommendationBet) => edits[betKey(b)]?.stake ?? b.stake;
  const effPayout = (b: RecommendationBet) => edits[betKey(b)]?.payout ?? 0;
  const totalStake = useMemo(
    () => bets.reduce((s, b) => s + effStake(b), 0),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [bets, edits],
  );
  const overBudget = totalStake > balance;

  const setEdit = (b: RecommendationBet, patch: Partial<Edit>) =>
    setEdits((prev) => {
      const k = betKey(b);
      const cur = prev[k] ?? { stake: b.stake, payout: 0 };
      return { ...prev, [k]: { ...cur, ...patch } };
    });

  const useSuggested = () => setEdits({}); // 推奨額に戻す（編集をクリア）。
  const skipAll = () =>
    setEdits(
      Object.fromEntries(bets.map((b) => [betKey(b), { stake: 0, payout: 0 }])),
    );

  const submit = () => {
    const payload: BetInput[] = bets
      .filter((b) => effStake(b) > 0)
      .map((b) => ({
        bet_type: b.bet_type,
        combination: b.combination,
        stake: effStake(b),
        payout: effPayout(b),
        ev: b.ev,
      }));
    record.mutate(payload); // 空配列 = スキップ（このレースを見送りとして記録）。
  };

  return (
    <section>
      <div className="toolbar">
        <h2>
          {card.data
            ? `${VENUE_JP[card.data.venue] ?? card.data.venue} ${card.data.race_num}R ${card.data.distance}m`
            : raceId}
        </h2>
        <Link to={`/?date=${date}`}>← レース一覧</Link>
        <Link to={`/sessions/${date}`}>収支</Link>
      </div>

      {session.isSuccess && session.data === null && (
        <p className="error">
          この開催日のセッションが未作成です。
          <Link to={`/sessions/${date}`}>収支ページ</Link>で開始してください。
        </p>
      )}
      {session.data && (
        <p className="session-balance">残高 {yen(balance)}</p>
      )}

      <h3>確率推定</h3>
      {prediction.isPending && <p>読み込み中…</p>}
      {prediction.isError && (
        <p className="error">{(prediction.error as Error).message}</p>
      )}
      {prediction.data && (
        <table className="grid">
          <thead>
            <tr>
              <th>馬番</th>
              <th>馬名</th>
              <th>勝率</th>
              <th>連対率</th>
              <th>複勝率</th>
            </tr>
          </thead>
          <tbody>
            {prediction.data.probabilities.map((p) => (
              <tr key={p.horse_num}>
                <td>{p.horse_num}</td>
                <td>{p.horse_name}</td>
                <td>{pct(p.win_prob)}</td>
                <td>{pct(p.place_prob)}</td>
                <td>{pct(p.show_prob)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <h3 style={{ marginTop: "1.5rem" }}>買い目推奨</h3>
      <div className="toolbar">
        <label>
          予算/R{" "}
          <input
            type="number"
            min={100}
            step={100}
            value={budget}
            onChange={(e) => setBudget(Number(e.target.value))}
          />
          円
        </label>
        <span className="muted">実上限 {yen(cap)}（min(予算, 残高)）</span>
      </div>

      {recs.isPending && cap > 0 && <p>読み込み中…</p>}
      {recs.isError && (
        <p className="error">{(recs.error as Error).message}</p>
      )}

      {recs.data && !recs.data.odds_available && (
        <div className="toolbar">
          <span className="muted">オッズ未取得 — 推奨を出せません。</span>
          <button
            onClick={() => oddsRefresh.mutate()}
            disabled={oddsRefresh.isPending}
          >
            最新取得
          </button>
          {oddsRefresh.isSuccess && !oddsRefresh.data.fetched && (
            <span className="error">オッズ未公開（取得できませんでした）</span>
          )}
          {oddsRefresh.isError && (
            <span className="error">{(oddsRefresh.error as Error).message}</span>
          )}
        </div>
      )}

      {recs.data && recs.data.odds_available && bets.length === 0 && (
        <p className="muted">該当なし（予算内で組める買い目がありません）。</p>
      )}

      {recs.data && bets.length > 0 && (
        <>
          {recs.data.axis != null && (
            <p className="muted">
              軸 {recs.data.axis} → 相手 {recs.data.partners.join(",")}
              {recs.data.roi != null && recs.data.hit_prob != null && (
                <>
                  {" "}
                  / 期待回収率 {(recs.data.roi * 100).toFixed(1)}% / 的中率{" "}
                  {(recs.data.hit_prob * 100).toFixed(1)}%
                </>
              )}
            </p>
          )}
          <table className="grid">
            <thead>
              <tr>
                <th>券種</th>
                <th>組合せ</th>
                <th>オッズ</th>
                <th>EV</th>
                <th>賭け金</th>
                <th>払戻</th>
              </tr>
            </thead>
            <tbody>
              {bets.map((b) => (
                <tr key={betKey(b)}>
                  <td>{b.bet_type}</td>
                  <td>{b.combination}</td>
                  <td>{b.odds == null ? "-" : b.odds.toFixed(1)}</td>
                  <td>{b.ev.toFixed(2)}</td>
                  <td>
                    <input
                      type="number"
                      min={0}
                      step={100}
                      value={effStake(b)}
                      onChange={(e) =>
                        setEdit(b, { stake: Number(e.target.value) })
                      }
                      style={{ width: "6rem" }}
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      min={0}
                      step={100}
                      value={effPayout(b)}
                      onChange={(e) =>
                        setEdit(b, { payout: Number(e.target.value) })
                      }
                      style={{ width: "6rem" }}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>

          <div className="toolbar" style={{ marginTop: "0.75rem" }}>
            <button onClick={useSuggested}>推奨通り</button>
            <button onClick={skipAll}>スキップ</button>
            <span className={overBudget ? "error" : "muted"}>
              賭け合計 {yen(totalStake)} / 残高 {yen(balance)}
            </span>
            <button
              onClick={submit}
              disabled={overBudget || record.isPending || !session.data}
            >
              記録する
            </button>
            {record.isError && (
              <span className="error">{(record.error as Error).message}</span>
            )}
          </div>
        </>
      )}
    </section>
  );
}
