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
}
