import { type RefObject } from "react";
import { type BoardHorse, markSymbol } from "../../lib/board";

// 馬書評（クリックで展開する詳細パネル。#411 で RaceBoard から抽出）。
// horse は親が selectedHorse から horses.find で解決した値（見つからなければ undefined）。
// undefined のときは何も描画しない（元の IIFE の早期 return 相当を props 側で表現）。
export function HorseDetailPanel({
  horse: h,
  onClose,
  closeBtnRef,
}: {
  horse: BoardHorse | undefined;
  onClose: () => void;
  closeBtnRef: RefObject<HTMLButtonElement | null>;
}) {
  if (!h) return null;
  return (
    <div
      className="horse-detail"
      id="horse-detail-panel"
      role="region"
      aria-label={`${h.horse_num} ${h.horse_name} の書評`}
      onKeyDown={(e) => {
        if (e.key === "Escape") onClose();
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
          onClick={onClose}
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
}
