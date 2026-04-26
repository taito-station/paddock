pub mod error;
pub mod horse_result;
pub mod race;
pub mod string;

pub use error::{Error, Result};
pub use horse_result::{
    FinishingPosition, GateNum, HorseName, HorseNum, HorseResult, JockeyName, TimeSeconds,
    TrainerName,
};
pub use race::{Race, RaceId, Surface, TrackCondition, Venue, Weather};
