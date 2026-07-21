use std::future::Future;

use paddock_domain::RacePayouts;

use crate::error::Result;
use crate::netkeiba_scraper::ResultRow;

/// レース結果ページ（`race/result.html`）から着順と確定払戻を **1 回の取得で両方** 得るポート（#381）。
///
/// 着順（[`ResultRow`]）と払戻（[`RacePayouts`]）は同一の結果ページ HTML に載るため、
/// [`PayoutFetcher`](crate::PayoutFetcher) の payout 取得と `fetch_race_result` の着順取得を
/// 別々に GET すると同一ページを二重取得してしまう。同日取り込み（`ResultsInteractor`）は
/// レースごとに 1 回だけ HTML を取得したいので、両方をまとめて返す専用ポートを設ける。
///
/// 実装（Interface 層）が HTTP 取得・charset デコード・HTML パースを担い、use-case 層はこの
/// トレイトだけに依存する。未確定（結果ページ未生成・払戻ブロック無し）なら払戻は空の
/// [`RacePayouts`]（`is_empty() == true`）を、着順は空 `Vec` を返す。
///
/// メソッドは **async**（#458）。実装は同期ブロッキング I/O（`std::thread::sleep` + 同期 ureq
/// GET）を伴うため、api-server の `results:refresh`（web が 45 秒間隔で自動ポーリング）経路では
/// 実装側で `tokio::task::spawn_blocking` に逃がし、未確定レース数ぶんの直列 sleep+GET で actix
/// worker を数十秒〜分単位で塞がないようにする。戻り値の Future に `Send` を課すのは actix-web の
/// マルチスレッドランタイム越しに握るため（conventions.md）。
pub trait ResultPageFetcher: Send + Sync {
    /// netkeiba 12 桁 race_id の結果ページを 1 回取得し、着順と確定払戻を返す。
    fn fetch_race_result_page(
        &self,
        netkeiba_race_id: &str,
    ) -> impl Future<Output = Result<(Vec<ResultRow>, RacePayouts)>> + Send;
}
