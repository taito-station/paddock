pub mod race_odds;

use crate::odds_scraper::OddsScraper;

/// レースのオッズを「保存済み(race_odds)参照 → 無ければライブスクレイプして保存」で取得する
/// read-through なユースケース。
///
/// ADR 0001/0005 は当初オッズの永続化を持たない設計だったが、予想の再現性と当時オッズでの
/// バックテストのため #51(ADR 0010) で永続化参照へ切り替えた。当初は単勝・複勝に限っていたが
/// #38 で組合せ券種(馬連・ワイド・馬単・3連複・3連単)も保存・再参照する。`OddsScraper`/`OddsRepository`
/// を必要とするため、メイン `Interactor<R, P, F>` には波及させず専用 interactor として切り出している。
pub struct OddsInteractor<O: OddsScraper, R> {
    pub scraper: O,
    pub repository: R,
}

impl<O: OddsScraper, R> OddsInteractor<O, R> {
    pub fn new(scraper: O, repository: R) -> Self {
        Self {
            scraper,
            repository,
        }
    }
}
