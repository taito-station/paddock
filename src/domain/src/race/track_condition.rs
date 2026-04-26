use strum_macros::Display;

use crate::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum TrackCondition {
    #[strum(to_string = "良")]
    Firm,
    #[strum(to_string = "稍重")]
    Good,
    #[strum(to_string = "重")]
    Yielding,
    #[strum(to_string = "不良")]
    Soft,
}

impl TrackCondition {
    pub fn as_str(&self) -> &'static str {
        match self {
            TrackCondition::Firm => "良",
            TrackCondition::Good => "稍重",
            TrackCondition::Yielding => "重",
            TrackCondition::Soft => "不良",
        }
    }
}

impl TryFrom<&str> for TrackCondition {
    type Error = Error;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.trim() {
            "良" => Ok(TrackCondition::Firm),
            "稍重" | "稍" => Ok(TrackCondition::Good),
            "重" => Ok(TrackCondition::Yielding),
            "不良" | "不" => Ok(TrackCondition::Soft),
            other => Err(Error::InvalidFormat(format!(
                "unknown track condition: {other}"
            ))),
        }
    }
}
