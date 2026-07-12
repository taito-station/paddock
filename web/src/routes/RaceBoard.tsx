import { useEffect, useRef, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import { pct, yen, SURFACE_JP, VENUE_JP } from "../lib/format";
import {
  DEFAULT_RACE_BUDGET,
  boardBudget,
  effectiveCap,
  keepBoardPlaceholder,
  heatColor,
  markSymbol,
  placeOddsLabel,
  sortByModelRank,
} from "../lib/board";
import { toAmount } from "../lib/bets";
import { boardHref } from "../lib/live";
import { ExecutionPanel } from "./board/ExecutionPanel";

export function RaceBoard() {
  const { raceId = "" } = useParams();
  const [searchParams] = useSearchParams();
  const dateParam = searchParams.get("date") ?? "";
  // ライブ日次ボード（/live/:date）から来たか。来た場合は戻り導線をライブに向け、
  // 盤内の場内/R切替でも from を引き継いで戻り先を保つ。
  const fromLive = searchParams.get("from") === "live";
  // クリックで馬書評（詳細パネル）を開く馬番。同じ馬を再クリック or 閉じるで null に戻す。
  const [selectedHorse, setSelectedHorse] = useState<number | null>(null);
  // フォーカス管理（a11y）: パネルを開いた馬カラム（trigger）を覚えておき、閉じたら戻す。
  // 開いたらパネル内（閉じるボタン）へフォーカスを移す。
  const triggerRef = useRef<HTMLElement | null>(null);
  const closeBtnRef = useRef<HTMLButtonElement | null>(null);
  // レース遷移（React Router の param 変更では remount されない）で開いたパネルが別馬に
  // 引き継がれるのを防ぐため、raceId が変わったら選択を解除する。
  useEffect(() => {
    setSelectedHorse(null);
  }, [raceId]);
  // パネルが開いたら閉じるボタンへフォーカス（キーボード操作でパネルに入れるように）。
  useEffect(() => {
    if (selectedHorse != null) closeBtnRef.current?.focus();
  }, [selectedHorse]);

  // パネルを閉じ、開いた馬カラムへフォーカスを戻す。
  const closePanel = () => {
    setSelectedHorse(null);
    triggerRef.current?.focus();
  };

  // 入力中の文字列と、盤/買い目の再計算に使う確定値を分離する。入力ごとの再取得
  //（重い盤 API）を避け、確定（blur / Enter / 再計算ボタン）で appliedBudget に反映する。
  const [budgetInput, setBudgetInput] = useState(String(DEFAULT_RACE_BUDGET));
  const [appliedBudget, setAppliedBudget] = useState(DEFAULT_RACE_BUDGET);
  const applyBudget = () => {
    const n = toAmount(budgetInput);
    if (n > 0) {
      if (n !== appliedBudget) setAppliedBudget(n);
      setBudgetInput(String(n)); // 入力を正規化（先頭ゼロ等）して表示と適用値を揃える。
    } else {
      // 不正入力（空・0 以下）は適用値へ巻き戻し、表示と適用値の乖離を残さない。
      setBudgetInput(String(appliedBudget));
    }
  };

  // 開催日は ?date= を優先。直リンク（クエリ無し）は盤レスポンスの date にフォールバック
  // するが、session（下）は board より先に宣言する必要があるため、フォールバック値は
  // board 到着後に state 経由で伝搬させる（board は budget=f(session) に依存する循環の解消）。
  const [fallbackDate, setFallbackDate] = useState("");
  // レース遷移で残留させない（別開催日の盤へ移った際、新 board 到着まで旧日付の
  // session を参照する transient の解消。?date= 無し直リンクのケースのみ影響）。
  useEffect(() => {
    setFallbackDate("");
  }, [raceId]);
  const sessionDate = dateParam || fallbackDate;

  // セッション（残高・記録済み判定）。未作成は 404 → null に倒す（RaceList と同流儀）。
  const session = useQuery({
    queryKey: ["session", sessionDate],
    enabled: !!sessionDate,
    retry: false,
    queryFn: async () => {
      const { data, error, response } = await api.GET("/api/sessions/{date}", {
        params: { path: { date: sessionDate } },
      });
      if (response.status === 404) return null;
      if (error) throw new Error("セッションの取得に失敗しました");
      return data;
    },
  });

  // 実効上限 cap = min(予算, 残高)。セッション無し（null/ロード中）は予算そのまま。
  // session ロード前は cap が予算のままなので、残高 < 予算 のときだけ session 到着後に
  // board の 2 回目フェッチが走る（軽微・許容）。
  const cap = effectiveCap(
    appliedBudget,
    session.data ? session.data.balance : null,
  );
  const queryBudget = boardBudget(cap);

  const board = useQuery({
    // budget は可変（#377）。stale キャッシュを避けるため queryKey に必ず含める。
    queryKey: ["board", raceId, queryBudget],
    enabled: !!raceId,
    // 予算変更時に盤全体（馬カラム）がスピナーへ戻るチラつきを防ぐ。ガードの意味論
    //（同一レース限定＝前レースの買い目を新レースとして記録できる事故の防止）は
    // keepBoardPlaceholder（lib/board.ts・テスト済み）が持つ。
    placeholderData: (prev, prevQuery) =>
      keepBoardPlaceholder(prevQuery?.queryKey, raceId) ? prev : undefined,
    queryFn: async () => {
      const { data, error } = await api.GET("/api/races/{race_id}/board", {
        params: {
          path: { race_id: raceId },
          query: { budget: queryBudget },
        },
      });
      if (error) throw new Error("盤の取得に失敗しました");
      return data;
    },
  });

  const date = dateParam || board.data?.date || "";
  // ?date= 無しの直リンクで盤が返した開催日を session 取得へ伝搬する。
  useEffect(() => {
    if (!dateParam && board.data?.date) setFallbackDate(board.data.date);
  }, [dateParam, board.data?.date]);

  // 同開催日の全レースを引き、同じ R の他場（函館⇄福島⇄小倉…）へ場内切替する。
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

  const d = board.data;
  const maxWin = d ? Math.max(0, ...d.horses.map((h) => h.win_prob)) : 0;
  const horses = d ? sortByModelRank(d.horses) : [];
  // 同じレース番号の他場（スラッグ辞書順で安定ソート）。
  const siblings = d
    ? (races.data?.races ?? [])
        .filter((r) => r.race_num === d.race_num)
        .sort((a, b) => a.venue.localeCompare(b.venue))
    : [];
  // 同じ開催場の各レース番号（1R→12R 昇順）。R 間のトグル移動用。
  const venueRaces = d
    ? (races.data?.races ?? [])
        .filter((r) => r.venue === d.venue)
        .sort((a, b) => a.race_num - b.race_num)
    : [];

  return (
    <section className="board-view">
      <div className="toolbar">
        <h2>
          {d
            ? `${VENUE_JP[d.venue] ?? d.venue} ${d.race_num}R ${SURFACE_JP[d.surface] ?? d.surface}${d.distance}m`
            : raceId}
        </h2>
        {d?.post_time && <span className="muted">発走 {d.post_time}</span>}
        {fromLive && date ? (
          <Link to={`/live/${date}`}>← ライブに戻る</Link>
        ) : (
          <Link to={`/?date=${date}`}>← レース一覧</Link>
        )}
        {date && <Link to={`/sessions/${date}`}>収支</Link>}
        {session.data && (
          <span className="session-balance">
            残高 {yen(session.data.balance)}
          </span>
        )}
        {session.isError && (
          <span className="error">セッションの取得に失敗しました</span>
        )}
      </div>

      {/* 同じ R の場内切替（函館⇄福島⇄小倉…） */}
      {d && siblings.length > 1 && (
        <div className="venue-switch">
          <span className="muted">{d.race_num}R:</span>
          {siblings.map((r) => {
            const label = VENUE_JP[r.venue] ?? r.venue;
            return r.race_id === d.race_id ? (
              <span key={r.race_id} className="chip venue-current">
                {label}
              </span>
            ) : (
              <Link
                key={r.race_id}
                className="chip venue-link"
                to={boardHref(r.race_id, date, { fromLive })}
              >
                {label}
              </Link>
            );
          })}
        </div>
      )}

      {/* 同じ開催場のレース番号トグル（1R⇄2R⇄…12R） */}
      {d && venueRaces.length > 1 && (
        <div className="venue-switch race-switch">
          <span className="muted">{VENUE_JP[d.venue] ?? d.venue}:</span>
          {venueRaces.map((r) =>
            r.race_id === d.race_id ? (
              <span key={r.race_id} className="chip venue-current">
                {r.race_num}
              </span>
            ) : (
              <Link
                key={r.race_id}
                className="chip venue-link"
                to={boardHref(r.race_id, date, { fromLive })}
              >
                {r.race_num}
              </Link>
            ),
          )}
        </div>
      )}

      {board.isPending && <p>読み込み中…</p>}
      {board.isError && <p className="error">{(board.error as Error).message}</p>}

      {d && (
        <>
          <div className="board-summary">
            {d.confusion.is_confused ? (
              <span className="chip chip-konsen">混戦</span>
            ) : (
              <span className="chip">平場</span>
            )}
            <span className="muted">
              ◎勝率 {pct(d.confusion.axis_win_prob)} ×0.70 以上{" "}
              {d.confusion.qualifying_count}頭
            </span>
            {d.axis != null && (
              <span className="muted">
                軸 {d.axis} → 相手 {d.partners.join(",")}
              </span>
            )}
            {d.roi != null && d.hit_prob != null && (
              <span className={d.roi >= 1 ? "chip chip-plus" : "muted"}>
                ROI {(d.roi * 100).toFixed(1)}% / 的中 {(d.hit_prob * 100).toFixed(1)}%
              </span>
            )}
            {!d.odds_available && (
              <span className="chip chip-warn">オッズ未取得</span>
            )}
          </div>

          {/* レース書評（混戦度・◎の狙いどころ・妙味。人手優先・無ければルールベース生成） */}
          {d.race_comment && (
            <p className="race-comment">{d.race_comment}</p>
          )}

          {/* 全頭横並び盤（モデル勝率順・truncate しない） */}
          <div className="board-scroll">
            <div className="board-row">
              {horses.map((h) => {
                // detail_lines はスキーマ上必須（string[]）。comment または根拠行があれば展開可。
                const hasDetail = !!h.comment || h.detail_lines.length > 0;
                // 開く時は trigger 要素を覚えてパネルからフォーカスを戻せるようにする。
                // ref 代入は updater の外（純粋な updater を保つ・StrictMode の二重実行対策）。
                const toggleDetail = (el: HTMLElement) => {
                  if (selectedHorse === h.horse_num) {
                    setSelectedHorse(null);
                  } else {
                    triggerRef.current = el;
                    setSelectedHorse(h.horse_num);
                  }
                };
                return (
                <div
                  key={h.horse_num}
                  className={
                    "horse-col" +
                    (h.is_overlay ? " is-overlay" : "") +
                    (h.is_value ? " is-value" : "") +
                    (hasDetail ? " has-detail" : "") +
                    (selectedHorse === h.horse_num ? " is-selected" : "")
                  }
                  role={hasDetail ? "button" : undefined}
                  tabIndex={hasDetail ? 0 : undefined}
                  aria-label={
                    hasDetail
                      ? `${h.horse_num} ${h.horse_name} の書評を開く`
                      : undefined
                  }
                  aria-expanded={hasDetail ? selectedHorse === h.horse_num : undefined}
                  aria-controls={
                    hasDetail && selectedHorse === h.horse_num
                      ? "horse-detail-panel"
                      : undefined
                  }
                  title={
                    hasDetail ? "クリック / Enter / Space で書評を表示" : undefined
                  }
                  onClick={
                    hasDetail ? (e) => toggleDetail(e.currentTarget) : undefined
                  }
                  onKeyDown={
                    hasDetail
                      ? (e) => {
                          if (e.key === "Enter" || e.key === " ") {
                            e.preventDefault();
                            toggleDetail(e.currentTarget);
                          }
                        }
                      : undefined
                  }
                >
                  <div
                    className="heat"
                    style={{ background: heatColor(h.win_prob, maxWin) }}
                    title={`モデル勝率 ${pct(h.win_prob)}`}
                  >
                    <span className="rank">{h.model_rank}</span>
                  </div>
                  <div className="num-mark">
                    <span className="num">{h.horse_num}</span>
                    <span className="mark">{markSymbol(h.mark)}</span>
                  </div>
                  <div className="hname" title={h.horse_name}>
                    {h.horse_name}
                  </div>
                  <div className="jockey">{h.jockey ?? "-"}</div>
                  <dl className="stats">
                    <div title="1着になる確率（勝率）">
                      <dt>勝率</dt>
                      <dd>{pct(h.win_prob)}</dd>
                    </div>
                    <div title="2着以内に入る確率（連対率）">
                      <dt>連対率</dt>
                      <dd>{pct(h.place_prob)}</dd>
                    </div>
                    <div title="3着以内に入る確率（複勝率）">
                      <dt>複勝率</dt>
                      <dd>{pct(h.show_prob)}</dd>
                    </div>
                    <div title="単勝オッズから逆算した市場推定の勝率（胴元の控除を抜いた実力評価）。モデル勝率と比べて乖離＝妙味">
                      <dt>市場勝率</dt>
                      <dd>{h.market_implied == null ? "-" : pct(h.market_implied)}</dd>
                    </div>
                    <div>
                      <dt>単勝</dt>
                      <dd>{h.win_odds == null ? "-" : h.win_odds.toFixed(1)}</dd>
                    </div>
                    <div>
                      <dt>複勝</dt>
                      <dd>{placeOddsLabel(h.place_odds_low, h.place_odds_high)}</dd>
                    </div>
                    <div>
                      <dt>人気</dt>
                      <dd>{h.popularity ?? "-"}</dd>
                    </div>
                  </dl>
                  <div className="flags">
                    {h.is_overlay && (
                      <span className="chip chip-overlay" title="モデル勝率1位×人気1位＝ほぼ複勝圏">
                        複勝圏
                      </span>
                    )}
                    {h.is_value && (
                      <span className="chip chip-value" title="モデル上位×市場人気低＝妙味・ワイドボックス候補">
                        妙味
                      </span>
                    )}
                    {hasDetail && <span className="chip chip-note">書評</span>}
                  </div>
                </div>
                );
              })}
            </div>
          </div>

          {/* 馬書評（クリックで展開する詳細パネル）。数値密度を保ちつつ掘りたい馬だけ開く */}
          {selectedHorse != null &&
            (() => {
              const h = horses.find((x) => x.horse_num === selectedHorse);
              if (!h) return null;
              return (
                <div
                  className="horse-detail"
                  id="horse-detail-panel"
                  role="region"
                  aria-label={`${h.horse_num} ${h.horse_name} の書評`}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") closePanel();
                  }}
                >
                  <div className="horse-detail-head">
                    <span className="mark">{markSymbol(h.mark)}</span>
                    <strong>
                      {h.horse_num} {h.horse_name}
                    </strong>
                    <span className="muted">{h.jockey ?? "-"}</span>
                    <button
                      ref={closeBtnRef}
                      className="detail-close"
                      onClick={closePanel}
                      aria-label="閉じる"
                    >
                      ×
                    </button>
                  </div>
                  {h.comment && <p className="horse-detail-lead">{h.comment}</p>}
                  {/* パネルは hasDetail(=comment もしくは detail_lines あり)でのみ開くため、
                      detail_lines 空のとき comment は必ず存在する（lead 表示済み・追加表示は不要）。 */}
                  {h.detail_lines.length > 0 && (
                    <ul className="horse-detail-lines">
                      {h.detail_lines.map((line, i) => (
                        <li key={`${i}-${line}`}>{line}</li>
                      ))}
                    </ul>
                  )}
                </div>
              );
            })()}

          {/* 買い目＋執行（/recommendations と同経路・相手 top5 不変。#377 で RaceDetail を統合） */}
          <h3 style={{ marginTop: "1.25rem" }}>買い目</h3>
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
            {session.data && (
              <span className="muted">
                実上限 {yen(cap)}（min(予算, 残高)）
              </span>
            )}
          </div>
          {/* key でレース遷移・予算変更時に編集状態（edits/mutation）を初期化する。 */}
          <ExecutionPanel
            key={`${raceId}:${cap}`}
            raceId={raceId}
            date={date}
            bets={d.bets}
            oddsAvailable={d.odds_available}
            session={session.data}
            sessionError={session.isError}
            refreshing={board.isPlaceholderData}
            cap={cap}
          />
          <p className="muted" style={{ marginTop: "0.5rem" }}>
            {d.field_size}頭立て。買い目の相手は top5 固定（相手は広げない）。全頭盤で妙味馬・複勝圏馬を手動で拾う。
          </p>
        </>
      )}
    </section>
  );
}
