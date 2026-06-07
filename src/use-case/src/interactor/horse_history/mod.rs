pub mod fetch;

use crate::netkeiba_scraper::NetkeibaScraper;
use crate::repository::Repository;

/// 当日出走馬の近走を netkeiba から取得して `results` に取り込むユースケース。
pub struct HorseHistoryInteractor<R: Repository, S: NetkeibaScraper> {
    pub repository: R,
    pub scraper: S,
}

impl<R: Repository, S: NetkeibaScraper> HorseHistoryInteractor<R, S> {
    pub fn new(repository: R, scraper: S) -> Self {
        Self {
            repository,
            scraper,
        }
    }
}
