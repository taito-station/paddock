use core::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use paddock_domain::{HorseName, JockeyName, Race, RaceCard, RaceId, RaceOdds, Surface, Venue};

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct GroupStat {
    pub label: String,
    pub starts: u32,
    pub wins: u32,
    pub places: u32,
    pub shows: u32,
}

impl GroupStat {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            starts: 0,
            wins: 0,
            places: 0,
            shows: 0,
        }
    }

    pub fn win_rate(&self) -> f64 {
        if self.starts == 0 {
            0.0
        } else {
            self.wins as f64 / self.starts as f64
        }
    }

    pub fn place_rate(&self) -> f64 {
        if self.starts == 0 {
            0.0
        } else {
            self.places as f64 / self.starts as f64
        }
    }

    pub fn show_rate(&self) -> f64 {
        if self.starts == 0 {
            0.0
        } else {
            self.shows as f64 / self.starts as f64
        }
    }
}

#[derive(Debug, Clone)]
pub struct HorseStatsRow {
    pub horse_name: String,
    pub by_surface: Vec<GroupStat>,
    pub by_distance_band: Vec<GroupStat>,
    pub by_gate_group: Vec<GroupStat>,
    pub by_track_condition: Vec<GroupStat>,
    pub by_popularity_band: Vec<GroupStat>,
    pub overall: GroupStat,
}

#[derive(Debug, Clone)]
pub struct CourseStatsRow {
    pub venue: String,
    pub distance: u32,
    pub surface: String,
    pub by_gate_group: Vec<GroupStat>,
}

#[derive(Debug, Clone)]
pub struct JockeyStatsRow {
    pub jockey_name: String,
    pub overall: GroupStat,
    pub by_surface: Vec<GroupStat>,
    pub by_gate_group: Vec<GroupStat>,
}

/// A successful fetch+ingest of a JRA meeting-day PDF, persisted so the same
/// meeting is not re-fetched on a later run (exclusive control).
#[derive(Debug, Clone)]
pub struct FetchRecord {
    pub source_key: String,
    pub url: String,
    pub races_saved: u32,
    pub horses_saved: u32,
    /// When the fetch+ingest happened. Set by the use-case layer so the gateway
    /// stays free of clock side effects (and tests can control it).
    pub fetched_at: DateTime<Utc>,
}

pub trait Repository: Send + Sync {
    fn save_race(&self, race: &Race) -> impl Future<Output = Result<()>> + Send;

    fn horse_stats(&self, name: &HorseName) -> impl Future<Output = Result<HorseStatsRow>> + Send;

    fn course_stats(
        &self,
        venue: Venue,
        distance: u32,
        surface: Surface,
    ) -> impl Future<Output = Result<CourseStatsRow>> + Send;

    fn jockey_stats(
        &self,
        name: &JockeyName,
    ) -> impl Future<Output = Result<JockeyStatsRow>> + Send;

    fn count_races(&self) -> impl Future<Output = Result<u64>> + Send;

    fn race_exists(&self, race_id: &RaceId) -> impl Future<Output = Result<bool>> + Send;

    /// Whether a meeting-day source key has already been ingested.
    fn fetch_history_contains(&self, source_key: &str)
    -> impl Future<Output = Result<bool>> + Send;

    /// Record a successful meeting-day fetch+ingest in the history table.
    fn record_fetch(&self, record: &FetchRecord) -> impl Future<Output = Result<()>> + Send;

    fn save_race_card(&self, card: &RaceCard) -> impl Future<Output = Result<()>> + Send;

    fn find_race_card(
        &self,
        race_id: &RaceId,
    ) -> impl Future<Output = Result<Option<RaceCard>>> + Send;

    /// 指定日に開催されるレース一覧を race_num 昇順で返す。
    /// 予想用途のため `results` は読み込まず空 Vec で返す。
    fn find_races_by_date(
        &self,
        date: NaiveDate,
    ) -> impl Future<Output = Result<Vec<Race>>> + Send;

    /// race_id に対応するオッズを返す。未取得の場合は `None`。
    fn find_race_odds(
        &self,
        race_id: &RaceId,
    ) -> impl Future<Output = Result<Option<RaceOdds>>> + Send;
}
