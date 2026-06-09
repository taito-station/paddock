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

#[cfg(test)]
mod tests {
    use super::TrackCondition;

    #[test]
    fn try_from_accepts_canonical_and_abbreviated_labels() {
        assert_eq!(
            TrackCondition::try_from("良").unwrap(),
            TrackCondition::Firm
        );
        assert_eq!(
            TrackCondition::try_from("稍重").unwrap(),
            TrackCondition::Good
        );
        assert_eq!(
            TrackCondition::try_from("稍").unwrap(),
            TrackCondition::Good
        );
        assert_eq!(
            TrackCondition::try_from("重").unwrap(),
            TrackCondition::Yielding
        );
        assert_eq!(
            TrackCondition::try_from("不良").unwrap(),
            TrackCondition::Soft
        );
        assert_eq!(
            TrackCondition::try_from("不").unwrap(),
            TrackCondition::Soft
        );
        // 前後空白は無視される。
        assert_eq!(
            TrackCondition::try_from(" 良 ").unwrap(),
            TrackCondition::Firm
        );
    }

    #[test]
    fn try_from_rejects_unknown_label() {
        assert!(TrackCondition::try_from("泥").is_err());
        assert!(TrackCondition::try_from("").is_err());
    }
}
