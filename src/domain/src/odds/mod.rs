mod bet_type;
mod combination;
mod odds_value;

pub use bet_type::BetType;
pub use combination::{OrderedPair, OrderedTriple, Pair, Triple};
pub use odds_value::{OddsValue, PlaceOdds};

use std::collections::HashMap;

use crate::horse_result::HorseNum;
use crate::race::RaceId;

/// All bet-type odds maps scraped for a single race.
///
/// Each map is keyed by the bet combination and holds the quoted odds. Maps are
/// independent: a pool that JRA has not published yet is simply left empty.
#[derive(Debug, Clone)]
pub struct RaceOdds {
    pub race_id: RaceId,
    /// 単勝
    pub win: HashMap<HorseNum, OddsValue>,
    /// 複勝 (low..high band per horse)
    pub place: HashMap<HorseNum, PlaceOdds>,
    /// 馬連
    pub quinella: HashMap<Pair, OddsValue>,
    /// 馬単
    pub exacta: HashMap<OrderedPair, OddsValue>,
    /// 三連複
    pub trio: HashMap<Triple, OddsValue>,
    /// 三連単
    pub trifecta: HashMap<OrderedTriple, OddsValue>,
}

impl RaceOdds {
    /// Create an empty odds set for a race; callers fill the per-bet-type maps.
    pub fn empty(race_id: RaceId) -> Self {
        Self {
            race_id,
            win: HashMap::new(),
            place: HashMap::new(),
            quinella: HashMap::new(),
            exacta: HashMap::new(),
            trio: HashMap::new(),
            trifecta: HashMap::new(),
        }
    }

    /// True when no bet type has any quoted odds.
    pub fn is_empty(&self) -> bool {
        self.win.is_empty()
            && self.place.is_empty()
            && self.quinella.is_empty()
            && self.exacta.is_empty()
            && self.trio.is_empty()
            && self.trifecta.is_empty()
    }
}
