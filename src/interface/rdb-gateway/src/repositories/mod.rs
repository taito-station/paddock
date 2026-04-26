mod course_stats;
mod horse_stats;
mod jockey_stats;
mod save_race;

use paddock_domain::{HorseName, JockeyName, Race, RaceId, Surface, Venue};
use paddock_use_case::Result as UcResult;
use paddock_use_case::repository::{CourseStatsRow, HorseStatsRow, JockeyStatsRow, Repository};

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
}
