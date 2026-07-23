import { useState } from "react";
import { Link } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  api,
  type BetInput,
  type RecommendationBet,
  type SessionSummary,
} from "../../api/client";
import { yen } from "../../lib/format";
import {
  type Edits,
  betKey,
  buildOutcomeBets,
  canRecordOutcome,
  effPayout,
  effStake,
  hasInvalidStakeUnit,
  isRaceRecorded,
  toAmount,
  totalStake as sumStake,
} from "../../lib/bets";

// 盤の買い目セクション＝執行パネル（#377 で RaceDetail から統合）。
// 手編集（賭け金/払戻）・記録/スキップ・オッズ最新取得を担う。
// 親（RaceBoard）が key={`${raceId}:${cap}`} でマウントするため、レース遷移・予算変更で
// edits / mutation 状態は初期化される（買い目集合が変わったときの孤児エントリ防止）。
export function ExecutionPanel({
  raceId,
  date,
  bets,
  oddsAvailable,
  session,
  sessionError = false,
  refreshing = false,
  cap,
}: {
  raceId: string;
  date: string;
  bets: RecommendationBet[];
  oddsAvailable: boolean;
  // undefined=ロード中 / null=セッション未作成（404）/ 値あり=作成済み
  session: SessionSummary | null | undefined;
  // 取得エラー時は「読込中…」を出し続けない（エラー表示はヘッダ側が担う）。
  sessionError?: boolean;
  // 盤が placeholder（旧予算の bets）を表示中か。key=raceId:cap の再マウント後も
  // 再フェッチ完了までは旧予算の bets prop が渡る窓があるため、その間は記録を止める。
  refreshing?: boolean;
  cap: number;
}) {
  const qc = useQueryClient();
  const [edits, setEdits] = useState<Edits>({});

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
      // 買い目は board レスポンスに含まれるため、budget 変種ごと prefix で無効化する。
      qc.invalidateQueries({ queryKey: ["board", raceId] });
      setEdits({}); // 買い目集合が変わるので編集をリセット（孤児エントリ防止）。
    },
  });

  const record = useMutation({
    mutationFn: async (outcome: BetInput[]) => {
      const { data, error } = await api.POST(
        "/api/sessions/{date}/races/{race_id}/outcome",
        { params: { path: { date, race_id: raceId } }, body: { bets: outcome } },
      );
      if (error)
        throw new Error("記録に失敗しました（残高超過・二重記録の可能性）");
      return data;
    },
    onSuccess: (data) => {
      // サーバ確定値でセッションを更新し、盤に留まる（RaceDetail の収支遷移は廃止。
      // ライブ経由の戻り導線・場/R 切替での「記録→次の R」運用を保つ）。#377
      qc.setQueryData(["session", date], data);
    },
  });

  const balance = session?.balance ?? 0;
  const bought = !!session && isRaceRecorded(session.bets, raceId);
  // 見送り記録済みか（#481）。スキップは outcome を空 bets で POST するとサーバが痕跡を保存し、
  // レスポンス（および GET summary）の skipped_race_ids に載る。これでリロード・再訪しても
  // 「見送り済み」を判別でき、record.isSuccess のローカル表示に頼らずに済む（旧既知制約の解消）。
  const skipped =
    !!session && (session.skipped_race_ids ?? []).includes(raceId);
  const total = sumStake(bets, edits);
  // 残高が実弾の制約。「予算/R」(appliedBudget) は推奨額の算出ヒントであって手編集の上限では
  // ないため、超過判定は残高基準にする（編集で残高内まで増やすのは許容）。
  const overBudget = total > balance;
  // 賭け金は 100 円単位（端数不可）。手編集で 150 円等が入ったら記録を止める（買い方ルール）。
  // 払戻は 10 円単位のため対象外。サーバ検証頼みにせず UI でも弾く（#412）。
  const stakeUnitError = hasInvalidStakeUnit(bets, edits);
  // 完了済み・記録済みは記録不可（バックエンドも 409 等で拒否するが UI でも無効化）。
  const canRecord = canRecordOutcome({
    hasSession: !!session,
    completed: session?.completed ?? false,
    bought,
    pending: record.isPending,
  });

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

  // --- 記録済み: session キャッシュの明細を読取専用で表示（board の bets は予算次第で
  //     変わるため出さない）。 ---
  if (bought) {
    const recorded = session!.bets.filter((b) => b.race_id === raceId);
    return (
      <>
        <div className="toolbar">
          <span className="chip chip-plus">購入済み</span>
          <Link to={`/sessions/${date}`}>収支を見る</Link>
        </div>
        <div className="table-scroll">
          <table className="grid">
            <thead>
              <tr>
                <th>券種</th>
                <th>組合せ</th>
                <th>賭け金</th>
                <th>払戻</th>
              </tr>
            </thead>
            <tbody>
              {recorded.map((b) => (
                <tr key={`${b.bet_type}-${b.combination}`}>
                  <td>{b.bet_type}</td>
                  <td>{b.combination}</td>
                  <td>{yen(b.stake)}</td>
                  <td>{yen(b.payout)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </>
    );
  }

  // --- 見送り記録済み: サーバ保存の痕跡（skipped_race_ids）で判定するため、リロード・再訪でも
  //     「見送り済み」を維持する（#481。旧: record.isSuccess のローカル表示で、リロードで未処理に
  //     戻る既知制約があった）。 ---
  if (skipped) {
    return (
      <div className="toolbar">
        <span className="chip">見送り済み</span>
        <Link to={`/sessions/${date}`}>収支を見る</Link>
      </div>
    );
  }

  // --- セッション文脈の縮退表示（undefined=ロード中 / null=未作成） ---
  const sessionNote =
    session === null ? (
      <p className="muted">
        セッション未作成のため閲覧のみ（記録するには
        <Link to={`/sessions/${date}`}>収支ページ</Link>で開始）。
      </p>
    ) : session === undefined && date && !sessionError ? (
      <p className="muted">セッション読込中…</p>
    ) : null;
  // ロード中（undefined）・未作成（null）は執行操作を出さない。
  const canOperate = !!session;

  if (canOperate && cap <= 0) {
    return (
      <div className="toolbar">
        <span className="muted">残高・予算が 0 のため推奨を出せません。</span>
        {canRecord && <button onClick={skipRace}>このレースをスキップ</button>}
        {record.isError && (
          <span className="error">{record.error.message}</span>
        )}
      </div>
    );
  }

  if (!oddsAvailable) {
    return (
      <>
        {sessionNote}
        <div className="toolbar">
          <span className="muted">オッズ未取得 — 推奨を出せません。</span>
          {canOperate && (
            <button
              onClick={() => oddsRefresh.mutate()}
              disabled={oddsRefresh.isPending}
            >
              最新取得
            </button>
          )}
          {canRecord && (
            <button onClick={skipRace}>このレースをスキップ</button>
          )}
          {oddsRefresh.isSuccess && !oddsRefresh.data.fetched && (
            <span className="error">オッズ未公開（取得できませんでした）</span>
          )}
          {oddsRefresh.isError && (
            <span className="error">{oddsRefresh.error.message}</span>
          )}
          {record.isError && (
            <span className="error">{record.error.message}</span>
          )}
        </div>
      </>
    );
  }

  if (bets.length === 0) {
    return (
      <>
        {sessionNote}
        <div className="toolbar">
          <span className="muted">
            該当なし（予算内で組める買い目がありません）。
          </span>
          {canRecord && (
            <button onClick={skipRace}>このレースをスキップ</button>
          )}
          {record.isError && (
            <span className="error">{record.error.message}</span>
          )}
        </div>
      </>
    );
  }

  return (
    <>
      {sessionNote}
      <div className="table-scroll">
        <table className="grid">
          <thead>
            <tr>
              <th>券種</th>
              <th>組合せ</th>
              <th>オッズ</th>
              <th>EV</th>
              <th>賭け金</th>
              {canOperate && <th>払戻</th>}
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
                  {canOperate ? (
                    <input
                      type="number"
                      min={0}
                      step={100}
                      value={effStake(b, edits)}
                      onChange={(e) =>
                        setEdit(b, { stake: toAmount(e.target.value) })
                      }
                      className="amount-input"
                    />
                  ) : (
                    yen(b.stake)
                  )}
                </td>
                {canOperate && (
                  <td>
                    <input
                      type="number"
                      min={0}
                      step={100}
                      value={effPayout(b, edits)}
                      onChange={(e) =>
                        setEdit(b, { payout: toAmount(e.target.value) })
                      }
                      className="amount-input"
                    />
                  </td>
                )}
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {canOperate && (
        <div className="toolbar mt-md">
          <button onClick={() => setEdits({})}>推奨通り</button>
          <button onClick={skipAll}>スキップ</button>
          <span className={overBudget ? "error" : "muted"}>
            賭け合計 {yen(total)} / 残高 {yen(balance)}
          </span>
          {stakeUnitError && (
            <span className="error">賭け金は 100 円単位で入力してください</span>
          )}
          {/* refreshing 中の bets は旧予算の placeholder のため記録を止める。
              上方の各分岐にある「このレースをスキップ」（skipRace＝空配列）は bets 非依存
              なので refreshing では無効化していない。 */}
          <button
            onClick={() => record.mutate(buildOutcomeBets(bets, edits))}
            disabled={overBudget || stakeUnitError || !canRecord || refreshing}
          >
            {refreshing
              ? "再計算中…"
              : total === 0
                ? "スキップとして記録"
                : "記録する"}
          </button>
          {record.isError && (
            <span className="error">{record.error.message}</span>
          )}
        </div>
      )}
    </>
  );
}
