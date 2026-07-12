import type { RaceBadge } from "../../lib/format";

// 購入状態バッジ（旧 RaceList から移設）。ライブ行・静的縮退行の両方から使う。
export function Badge({ kind }: { kind: RaceBadge }) {
  switch (kind) {
    case "bought":
      return <span className="badge badge-bought">購入済み</span>;
    case "skipped":
      return <span className="badge">見送り</span>;
    case "pending":
      return <span className="badge">未処理</span>;
    case "none":
      // セッション未作成時は購入状況が不明なのでバッジを出さない。
      return <span className="muted">-</span>;
  }
}
