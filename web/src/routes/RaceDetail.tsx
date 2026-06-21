import { useState } from "react";
import { useParams, useNavigate, Link } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, type BetInput, type RecommendationBet } from "../api/client";
import { pct, yen, VENUE_JP } from "../lib/format";
import {
  type Edits,
  betKey,
  buildOutcomeBets,
  effPayout,
  effStake,
  toAmount,
  totalStake as sumStake,
} from "../lib/bets";

const DEFAULT_RACE_BUDGET = 5000; // CLI predict の既定 race_budget と揃える。
// 本番モデルの市場ブレンド係数（モデル α=0.3 ＋ 市場 0.7）。CLI predict / live EV と揃える。
// これを渡さないと API は素のモデル確率を返し、画面の本命が買い目の本命と食い違う。
const PREDICT_BLEND_ALPHA = 0.3;

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
  // 入力中の文字列と、推奨計算に使う確定値を分離する。入力ごとの再取得（重い推奨計算）を避け、
  // 確定（blur / Enter / 再計算ボタン）で appliedBudget に反映する。
  const [budgetInput, setBudgetInput] = useState(String(DEFAULT_RACE_BUDGET));
  const [appliedBudget, setAppliedBudget] = useState(DEFAULT_RACE_BUDGET);
  const [edits, setEdits] = useState<Edits>({});

  const applyBudget = () => {
    const n = toAmount(budgetInput);
    if (n > 0 && n !== appliedBudget) {
      setAppliedBudget(n);
      setEdits({}); // 買い目集合が変わるので編集をリセット（孤児エントリ防止）。
    }
  };

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
      const { data, error } = await api.GET("/api/races/{race_id}/prediction", {
        params: {
          path: { race_id: raceId },
          query: { blend_alpha: PREDICT_BLEND_ALPHA },
        },
      });
      if (error) throw new Error("確率推定の取得に失敗しました");
      return data;
    },
  });

  const cap = Math.min(appliedBudget, balance);
  const recs = useQuery({
    queryKey: ["recommendations", raceId, cap],
    enabled: !!raceId && cap > 0,
    queryFn: async () => {
      const { data, error } = await api.GET(
        "/api/races/{race_id}/recommendations",
        {
          params: {
            path: { race_id: raceId },
            query: { budget: cap, blend_alpha: PREDICT_BLEND_ALPHA },
          },
        },
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
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["recommendations", raceId] });
      setEdits({}); // 買い目集合が変わるので編集をリセット（孤児エントリ防止）。
    },
  });

  const record = useMutation({
    mutationFn: async (bets: BetInput[]) => {
      const { data, error } = await api.POST(
        "/api/sessions/{date}/races/{race_id}/outcome",
        { params: { path: { date, race_id: raceId } }, body: { bets } },
      );
      if (error)
        throw new Error("記録に失敗しました（残高超過・二重記録の可能性）");
      return data;
    },
    onSuccess: (data) => {
      // サーバ確定値でセッションを更新し、収支ビューへ遷移する。
      qc.setQueryData(["session", date], data);
      navigate(`/sessions/${date}`);
    },
  });

  const bets = recs.data?.bets ?? [];
  const total = sumStake(bets, edits);
  // 残高が実弾の制約。「予算/R」(appliedBudget) は推奨額の算出ヒントであって手編集の上限では
  // ないため、超過判定は残高基準にする（編集で残高内まで増やすのは許容）。
  const overBudget = total > balance;
  // 完了済みセッションは記録不可（バックエンドも拒否するが UI でも無効化して無駄な往復を防ぐ）。
  const canRecord =
    !!session.data && !session.data.completed && !record.isPending;

  const setEdit = (b: RecommendationBet, patch: Partial<Edits[string]>) =>
    setEdits((prev) => {
      const k = betKey(b);
      const cur = prev[k] ?? { stake: b.stake, payout: 0 };
      return { ...prev, [k]: { ...cur, ...patch } };
    });

  const skipAll = () =>
    setEdits(
      Object.fromEntries(bets.map((b) => [betKey(b), { stake: 0, payout: 0 }])),
    );
  const skipRace = () => record.mutate([]); // 空 = このレースを見送りとして記録。

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

      {session.isError && (
        <p className="error">セッションの取得に失敗しました。</p>
      )}
      {session.isSuccess && session.data === null && (
        <p className="error">
          この開催日のセッションが未作成です。
          <Link to={`/sessions/${date}`}>収支ページ</Link>で開始してください。
        </p>
      )}
      {session.data && <p className="session-balance">残高 {yen(balance)}</p>}

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
            value={budgetInput}
            onChange={(e) => setBudgetInput(e.target.value)}
            onBlur={applyBudget}
            onKeyDown={(e) => e.key === "Enter" && applyBudget()}
          />
          円
        </label>
        <button onClick={applyBudget}>再計算</button>
        <span className="muted">実上限 {yen(cap)}（min(予算, 残高)）</span>
      </div>

      {session.isPending && <p>読み込み中…</p>}
      {session.data && cap <= 0 && (
        <p className="muted">残高・予算が 0 のため推奨を出せません。</p>
      )}
      {recs.isPending && cap > 0 && <p>読み込み中…</p>}
      {recs.isError && <p className="error">{(recs.error as Error).message}</p>}

      {recs.data && !recs.data.odds_available && (
        <div className="toolbar">
          <span className="muted">オッズ未取得 — 推奨を出せません。</span>
          <button
            onClick={() => oddsRefresh.mutate()}
            disabled={oddsRefresh.isPending}
          >
            最新取得
          </button>
          {canRecord && (
            <button onClick={skipRace}>このレースをスキップ</button>
          )}
          {oddsRefresh.isSuccess && !oddsRefresh.data.fetched && (
            <span className="error">オッズ未公開（取得できませんでした）</span>
          )}
          {oddsRefresh.isError && (
            <span className="error">{(oddsRefresh.error as Error).message}</span>
          )}
          {record.isError && (
            <span className="error">{(record.error as Error).message}</span>
          )}
        </div>
      )}

      {recs.data && recs.data.odds_available && bets.length === 0 && (
        <div className="toolbar">
          <span className="muted">
            該当なし（予算内で組める買い目がありません）。
          </span>
          {canRecord && (
            <button onClick={skipRace}>このレースをスキップ</button>
          )}
          {record.isError && (
            <span className="error">{(record.error as Error).message}</span>
          )}
        </div>
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
                      value={effStake(b, edits)}
                      onChange={(e) =>
                        setEdit(b, { stake: toAmount(e.target.value) })
                      }
                      style={{ width: "6rem" }}
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      min={0}
                      step={100}
                      value={effPayout(b, edits)}
                      onChange={(e) =>
                        setEdit(b, { payout: toAmount(e.target.value) })
                      }
                      style={{ width: "6rem" }}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>

          <div className="toolbar" style={{ marginTop: "0.75rem" }}>
            <button onClick={() => setEdits({})}>推奨通り</button>
            <button onClick={skipAll}>スキップ</button>
            <span className={overBudget ? "error" : "muted"}>
              賭け合計 {yen(total)} / 残高 {yen(balance)}
            </span>
            <button
              onClick={() => record.mutate(buildOutcomeBets(bets, edits))}
              disabled={overBudget || !canRecord}
            >
              {total === 0 ? "スキップとして記録" : "記録する"}
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
