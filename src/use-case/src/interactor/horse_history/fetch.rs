use std::collections::{HashMap, HashSet};

use paddock_domain::{HorseId, HorseResult, Race, RaceId};

use crate::dto::horse_history::fetch::FetchHorseHistoryResponse;
use crate::error::Result;
use crate::interactor::horse_history::HorseHistoryInteractor;
use crate::netkeiba_scraper::{HorsePastRun, NetkeibaScraper};
use crate::repository::Repository;

impl<R: Repository, S: NetkeibaScraper> HorseHistoryInteractor<R, S> {
    /// 出馬表(`race_ids`)と直接指定(`horse_ids`)の各馬について netkeiba の近走を取得し、
    /// netkeiba race_id 単位に集約して `results` に upsert する。
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
        for race_id in race_ids {
            // 出馬表 1 件の失敗で全体を止めず、warn してスキップ（個別馬の失敗と同じ扱い）。
            // 他 race_id と --horse-id 直接指定分の取り込みを救済する。
            let runners = match self.scraper.fetch_shutuba(race_id) {
                Ok(runners) => runners,
                Err(e) => {
                    tracing::warn!(race_id, error = %e, "出馬表取得に失敗、スキップ");
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

        // 2. 各馬の近走を取得し、netkeiba race_id 単位に Race へ集約。
        let mut races: HashMap<String, Race> = HashMap::new();
        let mut horses_fetched = 0;
        let mut horses_failed = 0;
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
            for run in runs {
                accumulate(&mut races, horse_id, run)?;
            }
        }

        // 3. 合成レースごとに upsert。
        let mut races_saved = 0;
        let mut results_saved = 0;
        for race in races.values() {
            self.repository.upsert_history_race(race).await?;
            races_saved += 1;
            results_saved += race.results.len();
        }

        Ok(FetchHorseHistoryResponse {
            horses_fetched,
            horses_failed,
            races_saved,
            results_saved,
        })
    }
}

/// 近走 1 走を該当馬の `HorseResult` として、合成 race_id `nk-<id>` の `Race` に積む。
/// 同一レース・同一馬番の重複は無視（DB の `UNIQUE(race_id, horse_num)` と整合）。
fn accumulate(
    races: &mut HashMap<String, Race>,
    horse_id: &HorseId,
    run: HorsePastRun,
) -> Result<()> {
    let key = run.netkeiba_race_id.clone();
    if !races.contains_key(&key) {
        let race_id = RaceId::try_from(format!("nk-{}", run.netkeiba_race_id))?;
        races.insert(
            key.clone(),
            Race {
                race_id,
                date: run.date,
                venue: run.venue,
                round: run.round,
                day: run.day,
                race_num: run.race_num,
                surface: run.surface,
                distance: run.distance,
                track_condition: run.track_condition,
                weather: None,
                results: Vec::new(),
            },
        );
    }
    let race = races.get_mut(&key).expect("inserted above");
    if race.results.iter().any(|r| r.horse_num == run.horse_num) {
        return Ok(());
    }
    race.results.push(HorseResult {
        finishing_position: run.finishing_position,
        status: run.status,
        gate_num: run.gate_num,
        horse_num: run.horse_num,
        horse_name: run.horse_name,
        horse_id: Some(horse_id.clone()),
        jockey: run.jockey,
        trainer: None,
        time_seconds: run.time_seconds,
        margin: run.margin,
        odds: run.odds,
        horse_weight: run.horse_weight,
        weight_change: run.weight_change,
        weight_carried: run.weight_carried,
        popularity: run.popularity,
    });
    Ok(())
}
