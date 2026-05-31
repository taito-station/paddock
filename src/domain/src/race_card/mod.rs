use crate::horse_result::{GateNum, HorseName, HorseNum, JockeyName};
use crate::race::{RaceId, Surface, Venue};

/// A single race's entry sheet (出馬表). Static pre-race information used as input for
/// pre-race tendency prediction; distinct from `Race` which carries day-of results.
#[derive(Debug, Clone)]
pub struct RaceCard {
    pub race_id: RaceId,
    pub venue: Venue,
    pub round: u32,
    pub day: u32,
    pub race_num: u32,
    pub surface: Surface,
    pub distance: u32,
    pub entries: Vec<HorseEntry>,
}

/// One horse entry in a race card. The minimum set required to look up tendencies
/// in the existing horse / jockey / course aggregations.
#[derive(Debug, Clone)]
pub struct HorseEntry {
    pub gate_num: GateNum,
    pub horse_num: HorseNum,
    pub horse_name: HorseName,
    pub jockey: Option<JockeyName>,
}
