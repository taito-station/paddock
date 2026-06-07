pub mod race_odds;

use crate::odds_scraper::OddsScraper;

/// レースのオッズを `OddsScraper` から都度取得するユースケース。
///
/// `#10`(ADR 0001) の方針どおりキャッシュ・永続化を持たず、呼び出しごとに
/// ライブスクレイプする。Repository を必要としないため `HorseHistoryInteractor`
/// と同じく専用 interactor として切り出し、メイン `Interactor<R, P, F>` に
/// `OddsScraper` ジェネリクスを波及させない（ADR 0001 決定 #4 を踏襲）。
pub struct OddsInteractor<O: OddsScraper> {
    pub scraper: O,
}

impl<O: OddsScraper> OddsInteractor<O> {
    pub fn new(scraper: O) -> Self {
        Self { scraper }
    }
}
