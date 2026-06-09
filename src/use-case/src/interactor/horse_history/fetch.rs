use std::collections::HashSet;

use paddock_domain::HorseId;

use crate::dto::horse_history::fetch::FetchHorseHistoryResponse;
use crate::error::Result;
use crate::interactor::horse_history::HorseHistoryInteractor;
use crate::netkeiba_scraper::{HorsePastRun, NetkeibaScraper};
use crate::repository::Repository;

impl<R: Repository, S: NetkeibaScraper> HorseHistoryInteractor<R, S> {
    /// 出馬表(`race_ids`)と直接指定(`horse_ids`)の各馬について netkeiba の近走を取得し、
    /// 馬単位で `horses` / `horse_past_runs` に upsert する。
    ///
    /// 1 馬の取得失敗は warn ログを出してスキップし、全体は続行する。
    pub async fn fetch_and_store(
        &self,
        race_ids: &[String],
        horse_ids: &[String],
    ) -> Result<FetchHorseHistoryResponse> {
        // 1. 取得対象の horse_id を集める（出馬表→各馬 + 直接指定）。出現順を保ち重複排除。
        let mut targets: Vec<HorseId> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut shutuba_failed = 0;
        for race_id in race_ids {
            // 出馬表 1 件の失敗で全体を止めず、warn してスキップ（個別馬の失敗と同じ扱い）。
            // 他 race_id と --horse-id 直接指定分の取り込みを救済する。
            let runners = match self.scraper.fetch_shutuba(race_id) {
                Ok(runners) => runners,
                Err(e) => {
                    tracing::warn!(race_id, error = %e, "出馬表取得に失敗、スキップ");
                    shutuba_failed += 1;
                    continue;
                }
            };
            for runner in runners {
                if seen.insert(runner.horse_id.value().to_string()) {
                    targets.push(runner.horse_id);
                }
            }
        }
        for raw in horse_ids {
            let id = HorseId::try_from(raw.clone())?;
            if seen.insert(id.value().to_string()) {
                targets.push(id);
            }
        }

        // 2. 各馬の近走を取得し、netkeiba_race_id で重複排除して horse 単位に upsert する。
        //    DB エラーは系統的障害とみなし fail-fast で中断する（upsert は冪等なので再実行で回復）。
        let mut horses_fetched = 0;
        let mut horses_failed = 0;
        let mut runs_saved = 0;
        for horse_id in &targets {
            let runs = match self.scraper.fetch_horse_history(horse_id) {
                Ok(runs) => runs,
                Err(e) => {
                    tracing::warn!(horse_id = %horse_id, error = %e, "近走取得に失敗、スキップ");
                    horses_failed += 1;
                    continue;
                }
            };
            horses_fetched += 1;

            // 同一馬が同一過去レースの行を複数返しても 1 走に集約する（DB の
            // `UNIQUE(horse_id, race_id)` と整合させ、件数を実際の保存数に合わせる）。
            let mut race_seen: HashSet<String> = HashSet::new();
            let deduped: Vec<HorsePastRun> = runs
                .into_iter()
                .filter(|r| race_seen.insert(r.netkeiba_race_id.clone()))
                .collect();
            if deduped.is_empty() {
                continue;
            }
            self.repository
                .upsert_horse_history(horse_id, &deduped)
                .await?;
            runs_saved += deduped.len();
        }

        Ok(FetchHorseHistoryResponse {
            horses_fetched,
            horses_failed,
            shutuba_failed,
            runs_saved,
        })
    }
}
