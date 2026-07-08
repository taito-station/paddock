import { useParams, Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { api, type LiveRaceView } from "../api/client";
import { VENUE_JP, yen } from "../lib/format";
import {
  BET_TYPE_JP,
  METHOD_JP,
  flipNotes,
  groupLegs,
  jstHm,
  maru,
  placeBand,
  roiPct,
  roughnessChip,
  skipReason,
  summaryLine,
  tierBadge,
} from "../lib/live";

const venueJp = (v: string) => VENUE_JP[v] ?? v;
const raceLabel = (r: LiveRaceView) => `${venueJp(r.venue)}${r.race_no}R`;

// 荒れ度チップ（ROI とは別軸＝分布の乱れ）。データが無ければ描画しない。#344
function RoughnessChip({ race }: { race: LiveRaceView }) {
  const chip = roughnessChip(race.roughness, race.roughness_label);
  return chip ? <span className="live-tag chip-rough">{chip}</span> : null;
}

// 🟢 張るレース: そのまま買える形（式別 / 方式 / 軸 / 相手 / 点数 / 金額）。
function BetCard({ race }: { race: LiveRaceView }) {
  const groups = groupLegs(race.slip.legs);
  const notes = flipNotes(race.flip, {
    axis: race.axis,
    roi: race.roi,
    verdict: race.verdict,
  });
  const flipped = notes.length > 0;
  return (
    <section className="live-card live-bet">
      <div className="live-card-head">
        {/* 張るレースは 🟢 を維持しつつ、フリップ時は 🔶 も併記（両シグナルを両立）。 */}
        <span className="live-mark">{flipped ? "🟢🔶" : "🟢"}</span>
        <strong>{raceLabel(race)}</strong>
        <span className="live-roi">ROI {roiPct(race.roi)}</span>
        <span>
          ◎{maru(race.axis)}（model {race.axis_prob.toFixed(0)}% 単勝
          {race.axis_win_odds != null ? race.axis_win_odds.toFixed(1) : "—"} 複勝
          {placeBand(race.axis_place_odds_low, race.axis_place_odds_high)}）
        </span>
        {race.konsen && <span className="live-tag">混戦</span>}
        <RoughnessChip race={race} />
        <span className="muted">発走 {race.post_time ?? "—"}</span>
      </div>

      {flipped &&
        notes.map((n) => (
          <p key={n} className="live-flip">
            🔶 {n}
          </p>
        ))}
      {race.odds_missing && (
        <p className="muted">
          ※ 一部買い目にオッズ欠落あり・ROI は過小評価の可能性
        </p>
      )}

      <table className="grid live-slip">
        <thead>
          <tr>
            <th>式別</th>
            <th>方式</th>
            <th>軸</th>
            <th>相手</th>
            <th>点数</th>
            <th>金額</th>
          </tr>
        </thead>
        <tbody>
          {groups.map((g) => (
            <tr key={`${g.betType}-${g.method}`}>
              <td>{BET_TYPE_JP[g.betType] ?? g.betType}</td>
              <td>
                {METHOD_JP[g.method] ?? g.method}
                {g.method === "box" && "（軸なし）"}
              </td>
              <td>{g.axis != null ? `◎${maru(g.axis)}` : "—"}</td>
              <td>{g.members.map(maru).join("")}</td>
              <td>{g.points}点</td>
              <td>{yen(g.amount)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

// 🟡惜しい / ⚪様子見レース: 段階ボードの 1 行（コンパクト）。買い(ROI≥100)ではないので伝票は出さず、
// tier バッジ・ROI・荒れ度・◎情報・フリップ注記を並べる（在庫は常に出すが「買い」に見せない）。#344
function StageRow({ race }: { race: LiveRaceView }) {
  const notes = flipNotes(race.flip, {
    axis: race.axis,
    roi: race.roi,
    verdict: race.verdict,
  });
  const flipped = notes.length > 0;
  return (
    <div className={`live-skip${flipped ? " live-flipped" : ""}`}>
      <span className="live-mark">{tierBadge(race.tier)}</span>
      <strong>{raceLabel(race)}</strong>
      <span className="muted">発走 {race.post_time ?? "—"}</span>
      <span className="live-roi">ROI {roiPct(race.roi)}</span>
      <RoughnessChip race={race} />
      <span>
        ◎{maru(race.axis)}（model {race.axis_prob.toFixed(0)}% 単勝
        {race.axis_win_odds != null ? race.axis_win_odds.toFixed(1) : "—"}）
      </span>
      {race.konsen && <span className="live-tag">混戦</span>}
      {/* 断然人気の見送り理由は −EV 局面の注意喚起として残す。 */}
      {race.axis_win_odds != null && race.axis_win_odds <= 1.9 && (
        <span className="muted">
          {skipReason({
            roi: race.roi,
            axis: race.axis,
            axis_win_odds: race.axis_win_odds,
          })}
        </span>
      )}
      {notes.map((n) => (
        <span key={n} className="live-flip">
          🔶 {n}
        </span>
      ))}
      {race.odds_missing && (
        <span className="muted">※ オッズ欠落・ROI 過小評価の可能性</span>
      )}
    </div>
  );
}

export function LiveBets() {
  const { date = "" } = useParams();

  const live = useQuery({
    queryKey: ["live", date],
    enabled: !!date,
    retry: false,
    queryFn: async () => {
      const { data, error } = await api.GET("/api/live/{date}", {
        params: { path: { date } },
      });
      if (error) throw new Error("ライブ EV の取得に失敗しました");
      return data;
    },
  });

  // ルート live/:date により date は通常必ず存在するが、空だと query が無効化され
  // isPending のまま「読み込み中…」で固まるため、派生計算の前に明示的にガードする。
  if (!date) {
    return <p className="error">開催日が指定されていません。</p>;
  }

  const races = live.data?.races ?? [];
  // 段階ボードは floor 未満（tier=hidden）を隠し、残りを ROI 降順の常時ランキングにする。
  // 🟢買い(ROI≥100)は伝票付き BetCard、🟡🄫⚪ は StageRow（在庫は常に出すが買いに見せない）。#344
  const visible = races
    .filter((r) => r.tier !== "hidden")
    .sort((a, b) => b.roi - a.roi);
  const hiddenCount = races.length - visible.length;

  return (
    <section>
      <div className="toolbar">
        <h2>ライブ EV {date}</h2>
        <Link to={`/?date=${date}`}>← レース一覧へ</Link>
        <button
          onClick={() => live.refetch()}
          disabled={live.isFetching}
        >
          更新
        </button>
        {live.data && (
          <span className="muted">
            最終更新 {jstHm(live.data.summary.last_updated)}
          </span>
        )}
      </div>

      {live.isPending && <p>読み込み中…</p>}
      {live.isError && <p className="error">{(live.error as Error).message}</p>}

      {live.data && (
        <>
          <p className="live-summary">{summaryLine(live.data.summary)}</p>

          {races.length === 0 ? (
            <p className="muted">監視データなし</p>
          ) : (
            <div className="live-section">
              {visible.map((r) =>
                r.tier === "buy" ? (
                  <BetCard key={r.race_id} race={r} />
                ) : (
                  <StageRow key={r.race_id} race={r} />
                ),
              )}
              {hiddenCount > 0 && (
                <p className="muted">
                  他 {hiddenCount} レースは当日 ROI 分布の下位（floor 未満）で非表示。
                </p>
              )}
            </div>
          )}
        </>
      )}
    </section>
  );
}
