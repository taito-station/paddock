import { type LiveRaceView } from "../../api/client";
import { yen } from "../../lib/format";
import {
  BET_TYPE_JP,
  METHOD_JP,
  groupLegs,
  maru,
  placeBand,
} from "../../lib/live";

// ダッシュボードテーブルの thead の <th> 本数。列を増減したらここも更新する
// （SlipRow の colSpan がズレると伝票展開行のレイアウトが崩れる）。
export const DASHBOARD_COLS = 9;

// 🟢買いレースの伝票（そのまま買える形: 式別 / 方式 / 軸 / 相手 / 点数 / 金額）。
// 買いにだけ出す（#344「在庫は常に出すが買いに見せない」）。
// 単勝はテーブル列から外したため、◎の単勝・複勝帯をここで補完する。
export function SlipRow({ race }: { race: LiveRaceView }) {
  const groups = groupLegs(race.slip.legs);
  return (
    <tr className="live-slip-row" id={`slip-${race.race_id}`}>
      <td colSpan={DASHBOARD_COLS}>
        <p className="muted live-slip-head">
          ◎{maru(race.axis)} 単勝{" "}
          {race.axis_win_odds != null ? race.axis_win_odds.toFixed(1) : "—"}{" "}
          複勝帯 {placeBand(race.axis_place_odds_low, race.axis_place_odds_high)}
        </p>
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
      </td>
    </tr>
  );
}
