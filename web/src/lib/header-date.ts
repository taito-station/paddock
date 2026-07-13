// ヘッダに常駐する開催日コンテキストの導出とナビ href 生成（純関数・ユニットテスト対象）。
// URL クエリ ?date= を状態の正とし、Layout がここで一度だけ date を解決して全画面へ引き継ぐ。

import { todayJst } from "./format";

// YYYY-MM-DD 形式か（live.ts postDate と同一正規形）。範囲検証はしない＝「形」だけ。
export function isIsoDate(v: string | null | undefined): v is string {
  return typeof v === "string" && /^\d{4}-\d{2}-\d{2}$/.test(v);
}

// ヘッダが表示・引き継ぐ「現在の開催日」を URL から導出する。
// 優先順: ?date= クエリ → /sessions/:date の path param → todayJst()。
// 不正形は無視して次段へフォールバックする。
export function currentHeaderDate(
  searchParams: URLSearchParams,
  pathname: string,
): string {
  const q = searchParams.get("date");
  if (isIsoDate(q)) return q;

  // SessionSummary は ?date= を持たず path param のみ。ここで拾わないと
  // 収支画面でヘッダの開催日が当日にリセットされてしまう。
  const m = /^\/sessions\/([^/]+)/.exec(pathname);
  if (m) {
    try {
      const seg = decodeURIComponent(m[1]);
      if (isIsoDate(seg)) return seg;
    } catch {
      // 不正なパーセントエンコーディングは無視して当日へフォールバック。
    }
  }

  return todayJst();
}

// ナビリンク href（選択中の開催日を ?date= で引き継ぐ）。値は encode して
// & / # 等によるクエリ注入を防ぐ（boardHref と同流儀）。
export function raceListHref(date: string): string {
  return `/?date=${encodeURIComponent(date)}`;
}

export function analyzeHref(date: string): string {
  return `/analyze?date=${encodeURIComponent(date)}`;
}
