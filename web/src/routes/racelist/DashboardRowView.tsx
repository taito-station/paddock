import { Link } from "react-router-dom";
import { VENUE_JP, type RaceBadge } from "../../lib/format";
import {
  evVisible,
  rowPostTime,
  surfaceDistance,
  type DashboardRow,
} from "../../lib/dashboard";
import {
  DANZEN_WIN_ODDS_MAX,
  boardHref,
  flipNotes,
  isSoon,
  maru,
  raceStarted,
  roiPct,
  roughnessChip,
  skipReason,
  tierShort,
} from "../../lib/live";
import { SlipRow } from "./SlipRow";
import { Badge } from "./Badge";

// ダッシュボードの 1 行（+ 買いなら伝票展開行）。行クリックで伝票トグル（リンクは除外）。
// EV 情報（ROI/軸/荒れ/伝票/フリップ）は evVisible=true のときだけ出す:
// live 無しは "—"、tier=hidden は「圏外」（数値を見せない = #344）。post_time は
// 無害な事実情報なので hidden でも表示・ソートに使う。
export function DashboardRowView({
  row,
  date,
  now,
  badge,
  slipOpen,
  onToggle,
}: {
  row: DashboardRow;
  date: string;
  now: Date;
  badge: RaceBadge;
  slipOpen: boolean;
  onToggle: () => void;
}) {
  const { race, live } = row;
  const showEv = evVisible(row);
  const post = rowPostTime(row);
  const started = raceStarted(date, post, now) === true;
  const soon = !started && isSoon(date, post, now);
  const notes =
    showEv && live
      ? flipNotes(live.flip, {
          axis: live.axis,
          roi: live.roi,
          verdict: live.verdict,
        })
      : [];
  const flipped = notes.length > 0;
  const isBuy = showEv && live?.tier === "buy";
  const chip = showEv && live ? roughnessChip(live.roughness, live.roughness_label) : null;
  const rowClass =
    [
      started ? "live-row-done" : "",
      soon ? "live-row-soon" : "",
      isBuy ? "live-row-buy" : "",
    ]
      .filter(Boolean)
      .join(" ") || undefined;

  return (
    <>
      <tr
        className={rowClass}
        onClick={(e) => {
          // レース名リンク等のクリックは行トグルにしない（買い行のみトグル対象）。
          if (!isBuy || (e.target as HTMLElement).closest("a")) return;
          onToggle();
        }}
      >
        <td className="live-status">
          {isBuy && live && (
            <button
              type="button"
              className="live-slip-toggle"
              aria-expanded={slipOpen}
              aria-controls={`slip-${race.race_id}`}
              aria-label={slipOpen ? "伝票を折りたたむ" : "伝票を展開"}
              onClick={(e) => {
                // 行クリックのトグルと二重発火させない。
                e.stopPropagation();
                onToggle();
              }}
            >
              {slipOpen ? "▼" : "▶"}
            </button>
          )}
          {started && "⚫終 "}
          {showEv && live ? (
            <>
              {tierShort(live.tier)}
              {flipped && "🔶"}
            </>
          ) : live ? (
            // snapshot にはあるが floor 未満。数値は見せず存在だけ示す。
            <span className="muted">圏外</span>
          ) : (
            <span className="muted">—</span>
          )}
        </td>
        <td>
          <Link
            className="live-race-link"
            to={boardHref(race.race_id, date)}
            title="全頭盤・top5理由を見る"
          >
            <strong>
              {VENUE_JP[race.venue] ?? race.venue}
              {race.race_num}R
            </strong>
          </Link>
        </td>
        <td>{post ?? "—"}</td>
        <td>{surfaceDistance(race.surface, race.distance)}</td>
        <td className={isBuy ? "live-roi" : undefined}>
          {showEv && live ? roiPct(live.roi) : "—"}
        </td>
        <td>
          {/* 単勝オッズを併記（ズレ増額 ADR 0060 の判断材料。専用列は幅の都合で置かない） */}
          {showEv && live
            ? `◎${maru(live.axis)} (${live.axis_prob.toFixed(0)}%)${
                live.axis_win_odds != null
                  ? ` @${live.axis_win_odds.toFixed(1)}`
                  : ""
              }`
            : "—"}
        </td>
        <td>
          {chip && <span className="live-tag chip-rough">{chip}</span>}
          {showEv && live?.konsen && <span className="live-tag">混戦</span>}
          {!chip && !(showEv && live?.konsen) && "—"}
        </td>
        <td>
          <Badge kind={badge} />
        </td>
        <td className="live-notes">
          {notes.map((n) => (
            <span key={n} className="live-flip">
              🔶 {n}
            </span>
          ))}
          {/* 断然人気の見送り理由は −EV 局面の注意喚起として残す（閾値は live.ts の共有定数）。 */}
          {showEv &&
            live &&
            !isBuy &&
            live.axis_win_odds != null &&
            live.axis_win_odds <= DANZEN_WIN_ODDS_MAX && (
              <span className="muted">
                {skipReason({
                  roi: live.roi,
                  axis: live.axis,
                  axis_win_odds: live.axis_win_odds,
                })}
              </span>
            )}
          {showEv && live?.odds_missing && (
            <span className="muted">※ オッズ欠落・ROI 過小評価の可能性</span>
          )}
        </td>
      </tr>
      {isBuy && live && slipOpen && <SlipRow race={live} />}
    </>
  );
}
