pub mod betting;
pub mod error;
pub mod horse_result;
pub mod odds;
pub mod prediction;
pub mod race;
pub mod race_card;
pub mod string;

pub use betting::{BetCombination, BettingConfig, BettingRecommendation, select_bets};
pub use error::{Error, Result};
pub use horse_result::{
    FinishingPosition, GateNum, HorseId, HorseName, HorseNum, HorseResult, JockeyName,
    ResultStatus, TimeSeconds, TrainerName,
};
pub use odds::{BetType, OddsValue, OrderedPair, OrderedTriple, Pair, PlaceOdds, RaceOdds, Triple};
pub use prediction::{HorseFactors, HorseProbability, RateTriple, estimate_probabilities};
pub use race::{Race, RaceId, Surface, TrackCondition, Venue, Weather};
pub use race_card::{HorseEntry, RaceCard};
