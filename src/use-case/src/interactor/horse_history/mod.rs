pub mod fetch;

use crate::netkeiba_scraper::NetkeibaScraper;
use crate::repository::HorseHistoryRepository;

/// 当日出走馬の近走を netkeiba から取得して `horses` / `horse_past_runs` に取り込むユースケース。
pub struct HorseHistoryInteractor<R, S: NetkeibaScraper> {
    pub repository: R,
    pub scraper: S,
}

impl<R: HorseHistoryRepository, S: NetkeibaScraper> HorseHistoryInteractor<R, S> {
    pub fn new(repository: R, scraper: S) -> Self {
        Self {
            repository,
            scraper,
        }
    }

    /// `horses` マスタを元に pdf 成績行の horse_id を backfill し、埋めた行数を返す。
    /// 近走取得（[`Self::fetch_and_store`]）で horses が更新された直後に呼ぶ想定。
    pub async fn backfill_horse_ids(&self) -> crate::error::Result<u64> {
        self.repository.backfill_results_horse_ids().await
    }
}
