import type { LiveQuery, SortKey } from "../../lib/live";

// ソート可能な列見出し。クリックで「同列=方向トグル / 別列=その列の既定方向」。
// 状態列だけは正準の固定順（sortRows が dir を反映しない）なので、方向表示を出さず
// aria-sort も "other" にする（▲/▼ と実際の並びの乖離を作らない）。
export function SortTh({
  label,
  col,
  query,
  onSort,
}: {
  label: string;
  col: SortKey;
  query: LiveQuery;
  onSort: (key: SortKey) => void;
}) {
  const active = query.sort === col;
  const fixedOrder = col === "status";
  const ariaSort = !active
    ? undefined
    : fixedOrder
      ? "other"
      : query.dir === "asc"
        ? "ascending"
        : "descending";
  return (
    <th aria-sort={ariaSort}>
      <button
        type="button"
        className={`live-sort${active ? " live-sort-active" : ""}`}
        onClick={() => onSort(col)}
      >
        {label}
        {active && !fixedOrder && (query.dir === "asc" ? " ▲" : " ▼")}
      </button>
    </th>
  );
}
