import { useEffect, useRef, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import { pct, yen, raceTitle, SURFACE_JP, VENUE_JP } from "../lib/format";
import {
  DEFAULT_RACE_BUDGET,
  boardBudget,
  effectiveCap,
  keepBoardPlaceholder,
  sortByModelRank,
} from "../lib/board";
import { isUnit100, toAmount } from "../lib/bets";
import { boardHref } from "../lib/live";
import { backToDashboardHref } from "../lib/dashboard";
import { useSessionQuery, useRacesQuery } from "../lib/queries";
import { ExecutionPanel } from "./board/ExecutionPanel";
import { HorseCard } from "./board/HorseCard";
import { HorseDetailPanel } from "./board/HorseDetailPanel";

export function RaceBoard() {
  const { raceId = "" } = useParams();
  const [searchParams] = useSearchParams();
  const dateParam = searchParams.get("date") ?? "";
  // ライブ一覧の絞り込み状態（sort/filter）。「← レース一覧」と場/R 切替リンクに引き継ぎ、
  // 盤から戻ったときにソート/フィルタを復元する（#380）。直リンク（back 無し）は空。
  const back = searchParams.get("back") ?? "";
  // 旧 ?from=live は /live 廃止（#378）で読まなくなった。付いていても無視される。
  // クリックで馬書評（詳細パネル）を開く馬番。同じ馬を再クリック or 閉じるで null に戻す。
  const [selectedHorse, setSelectedHorse] = useState<number | null>(null);
  // 純モデル（α=1.0）の 勝/連/複 を各馬カードに表示するか（#373）。既定 OFF＝ブレンド＋市場のみで
  // 情報過多を避ける。ON でモデル列（モ勝/モ連/モ複）を展開し、ブレンドとの乖離を読めるようにする。
  const [showModel, setShowModel] = useState(false);
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

  // 馬カード（HorseCard）のクリック/キー操作で書評パネルを開閉する。開く時は trigger 要素を
  // 覚えてパネルからフォーカスを戻せるようにする（ref 代入は state updater の外＝純粋な updater を
  // 保ち StrictMode の二重実行対策）。同じ馬の再選択で閉じる。
  const handleSelect = (horseNum: number, trigger: HTMLElement) => {
    if (selectedHorse === horseNum) {
      setSelectedHorse(null);
    } else {
      triggerRef.current = trigger;
      setSelectedHorse(horseNum);
    }
  };

  // 入力中の文字列と、盤/買い目の再計算に使う確定値を分離する。入力ごとの再取得
  //（重い盤 API）を避け、確定（blur / Enter / 再計算ボタン）で appliedBudget に反映する。
  const [budgetInput, setBudgetInput] = useState(String(DEFAULT_RACE_BUDGET));
  const [appliedBudget, setAppliedBudget] = useState(DEFAULT_RACE_BUDGET);
  // 予算/R が 100 円単位でないときの明示エラー（買い方ルール・#412）。黙って丸めず入力を残す。
  const [budgetUnitError, setBudgetUnitError] = useState(false);
  const applyBudget = () => {
    const n = toAmount(budgetInput);
    if (n > 0 && !isUnit100(n)) {
      // 100 円単位でない端数（150 等）は適用せず明示エラー。入力は残して修正を促す。
      setBudgetUnitError(true);
      return;
    }
    setBudgetUnitError(false);
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
  const sessionDate = dateParam || fallbackDate;

  // セッション（残高・記録済み判定）。未作成は 404 → null に倒す（RaceList と同流儀・#411 で共通化）。
  const session = useSessionQuery(sessionDate);

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
  // クリア（レース遷移時の旧日付 transient 解消）と再設定は単一 effect で行うこと:
  // 分離すると「raceId 変更＋キャッシュ済み board が同一開催日」のとき再設定側の deps が
  // 変わらず、fallbackDate が空のまま session が無効化されて固着する。raceId は
  // その再評価トリガーとして deps に含める。
  useEffect(() => {
    setFallbackDate(!dateParam && board.data?.date ? board.data.date : "");
  }, [raceId, dateParam, board.data?.date]);

  // 同開催日の全レースを引き、同じ R の他場（函館⇄福島⇄小倉…）へ場内切替する。
  const races = useRacesQuery(date);

  const d = board.data;
  const maxWin = d ? Math.max(0, ...d.horses.map((h) => h.win_prob)) : 0;
  const horses = d ? sortByModelRank(d.horses) : [];
  // 詳細パネル対象の馬を selectedHorse から解決（IIFE を解消し props で受け渡す・#411）。
  const selectedHorseData =
    selectedHorse != null
      ? horses.find((x) => x.horse_num === selectedHorse)
      : undefined;
  // 同じレース番号の他場（スラッグ辞書順で安定ソート）。venue slug は ASCII のため、
  // 他所（dashboard.ts / live.ts の並べ替え）と同じロケール非依存の単純比較で決定性を担保する。
  const siblings = d
    ? (races.data?.races ?? [])
        .filter((r) => r.race_num === d.race_num)
        .sort((a, b) => (a.venue < b.venue ? -1 : a.venue > b.venue ? 1 : 0))
    : [];
  // 同じ開催場の各レース番号（1R→12R 昇順）。R 間のトグル移動用。
  const venueRaces = d
    ? (races.data?.races ?? [])
        .filter((r) => r.venue === d.venue)
        .sort((a, b) => a.race_num - b.race_num)
    : [];

  // レース名(グレード)。raceTitle は名前が無ければ "" を返すので、前置スペースだけ条件付きにする。
  const headTitle = d ? raceTitle(d.race_name, d.race_class) : "";

  return (
    <section className="board-view">
      <div className="toolbar">
        <h2>
          {d
            ? `${VENUE_JP[d.venue] ?? d.venue} ${d.race_num}R ${SURFACE_JP[d.surface] ?? d.surface}${d.distance}m${
                headTitle ? ` ${headTitle}` : ""
              }`
            : raceId}
        </h2>
        {d?.post_time && <span className="muted">発走 {d.post_time}</span>}
        {/* 戻り先は盤の date と back=（ライブの絞り込み状態）を合成して復元する（#380）。
            date/back とも whitelist 再検証・エンコード込みで backToDashboardHref が組む。 */}
        <Link to={backToDashboardHref(back, date)}>← レース一覧</Link>
        {date && (
          <Link to={`/sessions/${encodeURIComponent(date)}`}>収支</Link>
        )}
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
                to={boardHref(r.race_id, date, back)}
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
                to={boardHref(r.race_id, date, back)}
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
            {/* 軸ロック（#388）: predict 記録済みの◎があれば買い目軸はそれに固定済み。ライブ再計算軸
                （live_axis＝市場ブレンド首位）と乖離したら警告し、直前オッズによる軸の無言フリップを可視化する
                （CLAUDE.md 軸ロック運用・ADR 0055/0060）。 */}
            {d.recorded_axis != null &&
              (d.live_axis != null && d.live_axis !== d.recorded_axis ? (
                <span
                  className="chip chip-danger"
                  title="買い目の軸は predict 記録済みの◎に固定しています。ライブ再計算（市場ブレンド）の首位はこれと異なりますが、軸ロック運用により差し替えません（乖離は増額判断の材料）。"
                >
                  ⚠ 記録軸 {d.recorded_axis} 固定（ライブ再計算は {d.live_axis}）
                </span>
              ) : (
                <span
                  className="chip"
                  title="買い目の軸は predict 記録済みの◎に固定しています（直前オッズで軸をフリップさせない）。"
                >
                  🔒 記録軸 {d.recorded_axis} 固定
                </span>
              ))}
            {d.roi != null && d.hit_prob != null && (
              <span className={d.roi >= 1 ? "chip chip-plus" : "muted"}>
                ROI {(d.roi * 100).toFixed(1)}% / 的中 {(d.hit_prob * 100).toFixed(1)}%
              </span>
            )}
            {!d.odds_available && (
              <span className="chip chip-danger">オッズ未取得</span>
            )}
          </div>

          {/* レース書評（混戦度・◎の狙いどころ・妙味。人手優先・無ければルールベース生成） */}
          {d.race_comment && (
            <p className="race-comment">{d.race_comment}</p>
          )}

          {/* 確率表示の切替（#373）: 既定はブレンド＋市場。ON で純モデル（モ勝/モ連/モ複）を各カードに展開。 */}
          <div className="board-controls">
            <label className="model-toggle">
              <input
                type="checkbox"
                checked={showModel}
                onChange={(e) => setShowModel(e.target.checked)}
              />
              モデル値（純 α=1.0）を表示
            </label>
          </div>

          {/* 全頭横並び盤（ブレンド勝率順＝model_rank・truncate しない） */}
          <div className="board-scroll">
            <div className="board-row">
              {horses.map((h) => (
                <HorseCard
                  key={h.horse_num}
                  horse={h}
                  maxWin={maxWin}
                  showModel={showModel}
                  isSelected={selectedHorse === h.horse_num}
                  onSelect={handleSelect}
                />
              ))}
            </div>
          </div>

          {/* 馬書評（クリックで展開する詳細パネル）。数値密度を保ちつつ掘りたい馬だけ開く。
              selectedHorse から対象馬を持ち上げて解決し（IIFE を解消）、パネル本体は
              board/HorseDetailPanel へ分離（#411）。未選択・未解決は panel 側で null 返し。 */}
          <HorseDetailPanel
            horse={selectedHorseData}
            onClose={closePanel}
            closeBtnRef={closeBtnRef}
          />

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
                // 入力を編集し始めたらエラーは即座に落とす（賭け金側のライブ挙動と揃える）。
                // 再検証は commit 境界（blur / Enter / 再計算）で applyBudget が行う。
                onChange={(e) => {
                  setBudgetInput(e.target.value);
                  if (budgetUnitError) setBudgetUnitError(false);
                }}
                onBlur={applyBudget}
                onKeyDown={(e) => e.key === "Enter" && applyBudget()}
              />
              円
            </label>
            <button onClick={applyBudget}>再計算</button>
            {budgetUnitError && (
              <span className="error">予算は 100 円単位で入力してください</span>
            )}
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
