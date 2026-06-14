pub mod backtest;
pub mod betting;
pub mod error;
pub mod horse_result;
mod normalize;
pub mod odds;
pub mod payout;
pub mod prediction;
pub mod race;
pub mod race_card;
pub mod simulation;
pub mod string;

pub use backtest::{
    BacktestReport, CalibrationMetrics, ExoticBet, ExoticSegment, FieldSizeSegment, HorseOutcome,
    PopularitySegment, RaceEvaluation, ReliabilityBin, SurfaceSegment, evaluate, exotic_segments,
};
pub use betting::{
    BetCombination, BettingConfig, BettingRecommendation, Podium, bet_hit, select_bets,
};
pub use error::{Error, Result};
pub use horse_result::{
    FinishingPosition, GateNum, HorseId, HorseName, HorseNum, HorseResult, JockeyName,
    ResultStatus, TimeSeconds, TrainerName,
};
pub use odds::{BetType, OddsValue, OrderedPair, OrderedTriple, Pair, PlaceOdds, RaceOdds, Triple};
pub use payout::{RacePayouts, Settlement, settle_bet};
pub use prediction::{
    DatedCounts, EstimationConfig, FactorStat, HorseFactors, HorseProbability, RateTriple,
    RecencyConfig, ShrinkageConfig, apply_recency_weight, blend_with_market_win,
    estimate_probabilities, estimate_probabilities_with_config, recent_form_score,
};
pub use race::{Race, RaceId, Surface, TrackCondition, Venue, Weather};
pub use race_card::{HorseEntry, RaceCard};
pub use simulation::{EvReport, Finish, Outcome, PlacedBet, SimInput, SimReport, simulate};
