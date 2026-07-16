import { Navigate, useParams, useSearchParams } from "react-router-dom";
import { boardHref } from "../lib/live";

// 旧 RaceDetail（#377 で盤に統合・廃止）のブックマーク・履歴互換リダイレクト。
export function LegacyRaceDetailRedirect() {
  const { date = "", raceId = "" } = useParams();
  return <Navigate to={boardHref(raceId, date)} replace />;
}

// 旧 /live/:date（#378 で日次ダッシュボードへ統合・廃止）のブックマーク互換。
// sort/filter クエリはスキーマが同一なのでそのまま引き継ぐ。
export function LegacyLiveRedirect() {
  const { date = "" } = useParams();
  const [sp] = useSearchParams();
  const params = new URLSearchParams(sp);
  if (date) params.set("date", date);
  const qs = params.toString();
  return <Navigate to={qs ? `/?${qs}` : "/"} replace />;
}
