pub mod backtest;
pub mod betting;
pub mod error;
pub mod horse_result;
mod normalize;
pub mod odds;
pub mod pad_prediction;
pub mod payout;
pub mod portfolio;
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
pub use pad_prediction::{Mark, PadPrediction, PredictionBet, PredictionHorse, PredictionResult};
pub use payout::{RacePayouts, Settlement, settle_bet};
pub use portfolio::{
    PairEvDiagnostic, PairEvDiagnostics, Portfolio, PortfolioBet, PortfolioConfig, build_portfolio,
    pair_ev_diagnostics,
};
pub use prediction::{
    DatedCounts, EstimationConfig, ExplainCategory, FactorExplanation, FactorStat,
    HorseExplanation, HorseFactors, HorseProbability, JockeyFormRun, PrevRunSummary,
    RECOMMENDED_MARKET_BLEND_ALPHA, RateTriple, RecencyConfig, RecentRun, ShrinkageConfig,
    StandardTimes, Verdict, apply_recency_weight, apply_win_power, blend_with_market_win,
    estimate_probabilities, estimate_probabilities_with_config, jockey_recent_form_score,
    recent_form_score,
};
pub use race::{Race, RaceId, Surface, TrackCondition, Venue, Weather};
pub use race_card::{HorseEntry, RaceCard};
pub use simulation::{EvReport, Finish, Outcome, PlacedBet, SimInput, SimReport, simulate};
