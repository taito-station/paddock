mod course_stats;
mod fetch_history;
mod find_race_card;
mod find_races_by_date;
mod horse_stats;
mod jockey_stats;
mod predict_session;
mod save_race;
mod save_race_card;
mod upsert_history_race;

use chrono::NaiveDate;
use paddock_domain::{HorseName, JockeyName, Race, RaceCard, RaceId, Surface, Venue};
use paddock_use_case::Result as UcResult;
use paddock_use_case::repository::{
    CourseStatsRow, FetchRecord, HorseStatsRow, JockeyStatsRow, PredictBetRecord,
    PredictSessionRecord, Repository,
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

    async fn upsert_history_race(&self, race: &Race) -> UcResult<()> {
        upsert_history_race::upsert_history_race(&self.pool, race)
            .await
            .map_err(Into::into)
    }

    async fn horse_stats(&self, name: &HorseName) -> UcResult<HorseStatsRow> {
        horse_stats::horse_stats(&self.pool, name)
            .await
            .map_err(Into::into)
    }

    async fn course_stats(
        &self,
        venue: Venue,
        distance: u32,
        surface: Surface,
    ) -> UcResult<CourseStatsRow> {
        course_stats::course_stats(&self.pool, venue, distance, surface)
            .await
            .map_err(Into::into)
    }

    async fn jockey_stats(&self, name: &JockeyName) -> UcResult<JockeyStatsRow> {
        jockey_stats::jockey_stats(&self.pool, name)
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
