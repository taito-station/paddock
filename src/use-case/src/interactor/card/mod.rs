pub mod ingest;

use crate::netkeiba_scraper::NetkeibaScraper;
use crate::repository::Repository;

/// netkeiba から当日の出馬表・単勝オッズを取得して保存するインタラクタ。
pub struct CardInteractor<R: Repository, S: NetkeibaScraper> {
    pub repo: R,
    pub scraper: S,
}

impl<R: Repository, S: NetkeibaScraper> CardInteractor<R, S> {
    pub fn new(repo: R, scraper: S) -> Self {
        Self { repo, scraper }
    }
}
