mod finishing_position;
mod gate_num;
mod horse_name;
mod horse_num;
mod jockey_name;
mod result_status;
mod time_seconds;
mod trainer_name;

pub use finishing_position::FinishingPosition;
pub use gate_num::GateNum;
pub use horse_name::HorseName;
pub use horse_num::HorseNum;
pub use jockey_name::JockeyName;
pub use result_status::ResultStatus;
pub use time_seconds::TimeSeconds;
pub use trainer_name::TrainerName;

#[derive(Debug, Clone)]
pub struct HorseResult {
    pub finishing_position: Option<FinishingPosition>,
    pub status: ResultStatus,
    pub gate_num: GateNum,
    pub horse_num: HorseNum,
    pub horse_name: HorseName,
    pub jockey: Option<JockeyName>,
    pub trainer: Option<TrainerName>,
    pub time_seconds: Option<TimeSeconds>,
    pub margin: Option<String>,
    pub odds: Option<f64>,
    pub horse_weight: Option<u32>,
    pub weight_change: Option<i32>,
    pub weight_carried: Option<f64>,
    pub popularity: Option<u32>,
}
