use paddock_domain::RacePayouts;

use crate::error::Result;

/// レース結果ページから確定払戻を取得するポート（#40）。
///
/// 実装（Interface 層）が HTTP 取得・EUC-JP デコード・HTML パースを担い、use-case 層は
/// このトレイトだけに依存する。`NetkeibaScraper` と同じく同期 I/O（ureq）で、未確定レースは
/// 空の [`RacePayouts`]（`is_empty() == true`）を返す。
pub trait PayoutFetcher: Send + Sync {
    /// netkeiba 12 桁 race_id のレース結果ページから確定払戻を取得する。
    /// 未確定（払戻ブロック無し）なら空の [`RacePayouts`] を返す。
    fn fetch_race_payouts(&self, netkeiba_race_id: &str) -> Result<RacePayouts>;
}
