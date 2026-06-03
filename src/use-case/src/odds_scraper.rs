use paddock_domain::{RaceId, RaceOdds};

use crate::error::Result;

/// Port for fetching live JRA betting odds for a single race.
///
/// Implementations (Interface layer) own the HTTP navigation and HTML parsing;
/// the use-case layer only depends on this trait. Odds are scraped on demand
/// per race with no caching.
pub trait OddsScraper: Send + Sync {
    fn scrape(&self, race_id: &RaceId) -> Result<RaceOdds>;
}
