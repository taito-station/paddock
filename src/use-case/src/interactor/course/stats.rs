use paddock_domain::{Surface, Venue};

use crate::error::Result;
use crate::interactor::Interactor;
use crate::repository::{CourseStatsRow, StatsRepository};

impl<R: StatsRepository> Interactor<R> {
    pub async fn course_stats(
        &self,
        venue: Venue,
        distance: u32,
        surface: Surface,
    ) -> Result<CourseStatsRow> {
        self.repository
            .course_stats(venue, distance, surface, None)
            .await
    }
}
