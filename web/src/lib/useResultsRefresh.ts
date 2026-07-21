import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import { isPastDate } from "./live";

// 当日の未確定レースが残る間だけ `POST /api/results/{date}:refresh`（force=false）をポーリングし、
// 着順取り込み＋自動精算を進める（#381・ADR 0068）。冪等 API なので read-through な useQuery として扱う。
//
// - `enabled`: 呼び出し側が「未精算が残るか（＝ポーリングすべきか）」を渡す。false になれば停止。
// - 過去日は常に停止（再現性重視・自動更新しない）。
// - 新規確定（`newly_confirmed_races > 0`）があれば races/live/session を無効化し、着順・払戻・
//   確定フラグを UI に反映する。全確定で `enabled` が false になりポーリングは自然停止する。
//
// 手動「精算」ボタン（`/sessions/{date}/results:refresh` エイリアス＝サーバ側 force=true）は
// フォールバックとして別途残す。
//
// 返り値 `isError`: ポーリングが失敗継続中か（retry:false なので直近試行の失敗をそのまま露出）。
// 呼び出し側はこれを消費して「自動精算が止まった」注記を出す（#478。無言停止の解消）。
//
// 注意: react-query v5 では `enabled=false`（停止）に戻しても一度 error になった query の
// `isError` は true のまま残る（自動リセットされない）。そのため「自動精算失敗 → 手動精算成功
// → 全確定で enabled=false」の経路で失敗注記が残存する実害があった。ここでは pollActive
// （＝ enabled 相当。未確定レースが残りポーリングすべき状態）が false のとき失敗を露出しない
// ガードを入れ、精算完了後に注記が確実に消えるようにする（#478 セルフレビュー1巡目）。
export function useResultsRefresh(
  date: string,
  { enabled, now }: { enabled: boolean; now: Date },
): { isError: boolean } {
  const qc = useQueryClient();
  // 過去日は常に停止。時刻源は呼び出し側の tick（now）に一元化する。
  const pollActive = enabled && !!date && !isPastDate(date, now);
  const { isError } = useQuery({
    queryKey: ["results-refresh", date],
    enabled: pollActive,
    retry: false,
    // 45 秒間隔。enabled が false になれば react-query が停止する（発火は enabled 中のみ）。
    refetchInterval: 45_000,
    queryFn: async () => {
      const { data, error } = await api.POST("/api/results/{date}:refresh", {
        params: { path: { date }, query: { force: false } },
      });
      if (error) throw new Error("結果の取り込みに失敗しました");
      if (data.newly_confirmed_races > 0) {
        void qc.invalidateQueries({ queryKey: ["races", date] });
        void qc.invalidateQueries({ queryKey: ["live", date] });
        void qc.invalidateQueries({ queryKey: ["session", date] });
      }
      return data;
    },
  });
  // ポーリング停止中（未確定なし＝精算完了、または過去日）は失敗を露出しない。
  // react-query は enabled=false で error 状態をリセットしないため、明示ガードで注記残存を防ぐ。
  return { isError: pollActive ? isError : false };
}
