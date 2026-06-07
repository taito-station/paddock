use strum_macros::Display;

use crate::error::Error;

/// The JRA bet types covered by the odds scraper and the bet simulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display)]
#[strum(serialize_all = "snake_case")]
pub enum BetType {
    /// 単勝
    Win,
    /// 複勝
    Place,
    /// 馬連
    Quinella,
    /// ワイド (拡大馬連: 2 頭がともに 3 着以内で的中)
    Wide,
    /// 馬単
    Exacta,
    /// 三連複
    Trio,
    /// 三連単
    Trifecta,
}

impl BetType {
    /// The Japanese label JRA uses for this bet type.
    pub fn as_ja(&self) -> &'static str {
        match self {
            BetType::Win => "単勝",
            BetType::Place => "複勝",
            BetType::Quinella => "馬連",
            BetType::Wide => "ワイド",
            BetType::Exacta => "馬単",
            BetType::Trio => "三連複",
            BetType::Trifecta => "三連単",
        }
    }
}

impl TryFrom<&str> for BetType {
    type Error = Error;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "win" | "単勝" | "WIN" => Ok(BetType::Win),
            "place" | "複勝" | "PLACE" => Ok(BetType::Place),
            "quinella" | "馬連" => Ok(BetType::Quinella),
            "wide" | "ワイド" | "WIDE" => Ok(BetType::Wide),
            "exacta" | "馬単" => Ok(BetType::Exacta),
            "trio" | "三連複" => Ok(BetType::Trio),
            "trifecta" | "三連単" => Ok(BetType::Trifecta),
            other => Err(Error::InvalidFormat(format!("unknown bet type: {other}"))),
        }
    }
}
