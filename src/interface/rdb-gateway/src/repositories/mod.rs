mod backfill_horse_ids;
mod course_stats;
mod fetch_history;
mod find_finished_races_between;
mod find_race_card;
mod find_races_by_date;
mod find_recent_runs;
mod horse_history;
mod horse_stats;
mod jockey_stats;
mod predict_session;
mod save_race;
mod save_race_card;
mod save_race_odds;
mod sql;

use chrono::NaiveDate;
use paddock_domain::{
    HorseId, HorseName, HorseResult, JockeyName, Race, RaceCard, RaceId, Surface, Venue,
};
use paddock_use_case::Result as UcResult;
use paddock_use_case::repository::{
    CourseStatsRow, FetchRecord, HorseStatsRow, JockeyStatsRow, PredictBetRecord,
    PredictSessionRecord, RaceOddsRecord, Repository,
};

use crate::pool::SqlitePool;

pub struct SqliteRepository {
    pub pool: SqlitePool,
}

impl SqliteRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl Repository for SqliteRepository {
    async fn save_race(&self, race: &Race) -> UcResult<()> {
        save_race::save_race(&self.pool, race)
            .await
            .map_err(Into::into)
    }

    async fn upsert_horse_history(
        &self,
        horse_id: &HorseId,
        runs: &[paddock_use_case::HorsePastRun],
    ) -> UcResult<()> {
        horse_history::upsert_horse_history(&self.pool, horse_id, runs)
            .await
            .map_err(Into::into)
    }

    async fn backfill_results_horse_ids(&self) -> UcResult<u64> {
        backfill_horse_ids::backfill_results_horse_ids(&self.pool)
            .await
            .map_err(Into::into)
    }

    async fn horse_stats(
        &self,
        name: &HorseName,
        as_of: Option<NaiveDate>,
    ) -> UcResult<HorseStatsRow> {
        horse_stats::horse_stats(&self.pool, name, as_of)
            .await
            .map_err(Into::into)
    }

    async fn course_stats(
        &self,
        venue: Venue,
        distance: u32,
        surface: Surface,
        as_of: Option<NaiveDate>,
    ) -> UcResult<CourseStatsRow> {
        course_stats::course_stats(&self.pool, venue, distance, surface, as_of)
            .await
            .map_err(Into::into)
    }

    async fn jockey_stats(
        &self,
        name: &JockeyName,
        as_of: Option<NaiveDate>,
    ) -> UcResult<JockeyStatsRow> {
        jockey_stats::jockey_stats(&self.pool, name, as_of)
            .await
            .map_err(Into::into)
    }

    async fn count_races(&self) -> UcResult<u64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM races")
            .fetch_one(&self.pool)
            .await
            .map_err(crate::Error::from)?;
        Ok(row.0 as u64)
    }

    async fn race_exists(&self, race_id: &RaceId) -> UcResult<bool> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM races WHERE race_id = $1 LIMIT 1")
            .bind(race_id.value())
            .fetch_optional(&self.pool)
            .await
            .map_err(crate::Error::from)?;
        Ok(row.is_some())
    }

    async fn fetch_history_contains(&self, source_key: &str) -> UcResult<bool> {
        fetch_history::contains(&self.pool, source_key)
            .await
            .map_err(Into::into)
    }

    async fn record_fetch(&self, record: &FetchRecord) -> UcResult<()> {
        fetch_history::record(&self.pool, record)
            .await
            .map_err(Into::into)
    }

    async fn save_race_card(&self, card: &RaceCard) -> UcResult<()> {
        save_race_card::save_race_card(&self.pool, card)
            .await
            .map_err(Into::into)
    }

    async fn save_race_odds(&self, record: &RaceOddsRecord) -> UcResult<()> {
        save_race_odds::save_race_odds(&self.pool, record)
            .await
            .map_err(Into::into)
    }

    async fn find_race_card(&self, race_id: &RaceId) -> UcResult<Option<RaceCard>> {
        find_race_card::find_race_card(&self.pool, race_id)
            .await
            .map_err(Into::into)
    }

    async fn find_races_by_date(&self, date: NaiveDate) -> UcResult<Vec<Race>> {
        find_races_by_date::find_races_by_date(&self.pool, date)
            .await
            .map_err(Into::into)
    }

    async fn find_finished_races_between(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> UcResult<Vec<Race>> {
        find_finished_races_between::find_finished_races_between(&self.pool, from, to)
            .await
            .map_err(Into::into)
    }

    async fn find_recent_runs(
        &self,
        name: &HorseName,
        before: NaiveDate,
        limit: u32,
    ) -> UcResult<Vec<(NaiveDate, HorseResult)>> {
        find_recent_runs::find_recent_runs(&self.pool, name, before, limit)
            .await
            .map_err(Into::into)
    }

    async fn find_predict_session(
        &self,
        date: NaiveDate,
    ) -> UcResult<Option<PredictSessionRecord>> {
        predict_session::find_predict_session(&self.pool, date)
            .await
            .map_err(Into::into)
    }

    async fn find_predict_bets(&self, date: NaiveDate) -> UcResult<Vec<PredictBetRecord>> {
        predict_session::find_predict_bets(&self.pool, date)
            .await
            .map_err(Into::into)
    }

    async fn save_predict_session(&self, session: &PredictSessionRecord) -> UcResult<()> {
        predict_session::save_predict_session(&self.pool, session)
            .await
            .map_err(Into::into)
    }

    async fn save_race_outcome(
        &self,
        session: &PredictSessionRecord,
        race_id: &RaceId,
        bets: &[PredictBetRecord],
    ) -> UcResult<()> {
        predict_session::save_race_outcome(&self.pool, session, race_id, bets)
            .await
            .map_err(Into::into)
    }
}
