mod race_id;
mod surface;
mod track_condition;
mod venue;
mod weather;

pub use race_id::RaceId;
pub use surface::Surface;
pub use track_condition::TrackCondition;
pub use venue::Venue;
pub use weather::Weather;

use chrono::NaiveDate;

use crate::horse_result::HorseResult;

#[derive(Debug, Clone)]
pub struct Race {
    pub race_id: RaceId,
    pub date: NaiveDate,
    pub venue: Venue,
    pub round: u32,
    pub day: u32,
    pub race_num: u32,
    pub surface: Surface,
    pub distance: u32,
    pub track_condition: Option<TrackCondition>,
    pub weather: Option<Weather>,
    pub results: Vec<HorseResult>,
}
