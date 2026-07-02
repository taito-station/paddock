use paddock_domain::{RaceId, RaceOdds};

use crate::error::Result;

/// Port for fetching live betting odds for a single race.
///
/// Implementations (Interface layer) own the HTTP fetch and response parsing;
/// the use-case layer only depends on this trait. Odds are scraped on demand
/// per race with no caching. The live implementation is `UreqNetkeibaScraper`
/// over the netkeiba odds API (UTF-8 JSON); the former JRA `accessO.html` path
/// was unverified (ADR 0001) and removed in #287.
pub trait OddsScraper: Send + Sync {
    fn scrape(&self, race_id: &RaceId) -> Result<RaceOdds>;

    /// 単勝・複勝**のみ**を取得する軽量経路（オッズ時系列コレクタ用・#odds-collect）。
    ///
    /// コレクタは全レースを終日・高頻度でスナップするため、組合せ 5 券種まで取る `scrape`
    /// （1 レース 6 GET）は重い。単勝中心の movement 収集では win/place（type=1・1 GET）で足りる。
    ///
    /// デフォルト実装は `scrape` の結果から win/place だけを残す（正しいが軽量でない）。
    /// ネットワーク実装（`UreqNetkeibaScraper`）は type=1 の 1 GET だけを打つよう **override** する。
    fn scrape_win_place(&self, race_id: &RaceId) -> Result<RaceOdds> {
        let full = self.scrape(race_id)?;
        let mut wp = RaceOdds::empty(race_id.clone());
        wp.win = full.win;
        wp.place = full.place;
        Ok(wp)
    }
}
