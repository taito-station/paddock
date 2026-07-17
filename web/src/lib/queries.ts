import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";

// セッション（残高・記録済み判定）を引く共有クエリ。未作成は 404 → null に倒し、
// それ以外の障害は握り潰さず投げる（500・ネットワーク断を「未作成」と取り違えない）。
// queryKey ["session", date] は RaceList / RaceBoard / SessionSummary の 3 画面で共有され
// キャッシュ整合が取れている。散在していた fetch 定義をここに一本化する（#411）。
// errorMessage は 404 以外の障害時に投げる文言。画面ごとの表記差（「セッション収支の…」等）を
// 保つため引数化し、既定は汎用文言。
export function useSessionQuery(
  date: string,
  errorMessage = "セッションの取得に失敗しました",
) {
  return useQuery({
    queryKey: ["session", date],
    enabled: !!date,
    retry: false,
    queryFn: async () => {
      const { data, error, response } = await api.GET("/api/sessions/{date}", {
        params: { path: { date } },
      });
      if (response.status === 404) return null;
      if (error) throw new Error(errorMessage);
      return data;
    },
  });
}

// 開催日の全レース一覧（DB 正本）を引く共有クエリ。queryKey ["races", date] は 3 画面で共有。
// 場内切替・ポーリング gate・静的一覧の突合に使う。散在していた fetch 定義を一本化する（#411）。
export function useRacesQuery(date: string) {
  return useQuery({
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
}
