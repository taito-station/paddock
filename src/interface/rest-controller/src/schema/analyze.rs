use serde::Serialize;
use utoipa::ToSchema;

use paddock_use_case::repository::{
    CourseStatsRow, GroupStat, HorseStatsRow, JockeyStatsRow, TrainerStatsRow,
};

/// あるカテゴリ 1 区分の成績（出走数＋勝/連対/複勝の件数とレート）。
#[derive(Debug, Serialize, ToSchema)]
pub struct GroupStatSchema {
    pub label: String,
    pub starts: u32,
    pub wins: u32,
    pub places: u32,
    pub shows: u32,
    pub win_rate: f64,
    pub place_rate: f64,
    pub show_rate: f64,
}

impl From<&GroupStat> for GroupStatSchema {
    fn from(g: &GroupStat) -> Self {
        Self {
            label: g.label.clone(),
            starts: g.starts,
            wins: g.wins,
            places: g.places,
            shows: g.shows,
            win_rate: g.win_rate(),
            place_rate: g.place_rate(),
            show_rate: g.show_rate(),
        }
    }
}

fn map_stats(rows: &[GroupStat]) -> Vec<GroupStatSchema> {
    rows.iter().map(GroupStatSchema::from).collect()
}

/// `GET /api/analyze/horse?name=` のレスポンス。
#[derive(Debug, Serialize, ToSchema)]
pub struct HorseStatsResponse {
    pub horse_name: String,
    pub overall: GroupStatSchema,
    pub by_surface: Vec<GroupStatSchema>,
    pub by_distance_band: Vec<GroupStatSchema>,
    pub by_gate_group: Vec<GroupStatSchema>,
    pub by_track_condition: Vec<GroupStatSchema>,
    pub by_popularity_band: Vec<GroupStatSchema>,
}

impl From<HorseStatsRow> for HorseStatsResponse {
    fn from(r: HorseStatsRow) -> Self {
        Self {
            horse_name: r.horse_name,
            overall: GroupStatSchema::from(&r.overall),
            by_surface: map_stats(&r.by_surface),
            by_distance_band: map_stats(&r.by_distance_band),
            by_gate_group: map_stats(&r.by_gate_group),
            by_track_condition: map_stats(&r.by_track_condition),
            by_popularity_band: map_stats(&r.by_popularity_band),
        }
    }
}

/// `GET /api/analyze/course?venue=&distance=&surface=` のレスポンス。
#[derive(Debug, Serialize, ToSchema)]
pub struct CourseStatsResponse {
    pub venue: String,
    pub distance: u32,
    pub surface: String,
    pub by_gate_group: Vec<GroupStatSchema>,
}

impl From<CourseStatsRow> for CourseStatsResponse {
    fn from(r: CourseStatsRow) -> Self {
        Self {
            venue: r.venue,
            distance: r.distance,
            surface: r.surface,
            by_gate_group: map_stats(&r.by_gate_group),
        }
    }
}

/// `GET /api/analyze/jockey?name=` のレスポンス。
#[derive(Debug, Serialize, ToSchema)]
pub struct JockeyStatsResponse {
    pub jockey_name: String,
    pub overall: GroupStatSchema,
    pub by_surface: Vec<GroupStatSchema>,
    pub by_gate_group: Vec<GroupStatSchema>,
}

impl From<JockeyStatsRow> for JockeyStatsResponse {
    fn from(r: JockeyStatsRow) -> Self {
        Self {
            jockey_name: r.jockey_name,
            overall: GroupStatSchema::from(&r.overall),
            by_surface: map_stats(&r.by_surface),
            by_gate_group: map_stats(&r.by_gate_group),
        }
    }
}

/// `GET /api/analyze/trainer?name=` のレスポンス。
#[derive(Debug, Serialize, ToSchema)]
pub struct TrainerStatsResponse {
    pub trainer_name: String,
    pub overall: GroupStatSchema,
    pub by_surface: Vec<GroupStatSchema>,
    pub by_gate_group: Vec<GroupStatSchema>,
}

impl From<TrainerStatsRow> for TrainerStatsResponse {
    fn from(r: TrainerStatsRow) -> Self {
        Self {
            trainer_name: r.trainer_name,
            overall: GroupStatSchema::from(&r.overall),
            by_surface: map_stats(&r.by_surface),
            by_gate_group: map_stats(&r.by_gate_group),
        }
    }
}
