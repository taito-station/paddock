use strum_macros::Display;

use crate::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[strum(serialize_all = "lowercase")]
pub enum Surface {
    Turf,
    Dirt,
}

impl Surface {
    pub fn as_str(&self) -> &'static str {
        match self {
            Surface::Turf => "turf",
            Surface::Dirt => "dirt",
        }
    }
}

impl TryFrom<&str> for Surface {
    type Error = Error;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "turf" | "TURF" | "芝" => Ok(Surface::Turf),
            "dirt" | "DIRT" | "ダート" | "ダ" => Ok(Surface::Dirt),
            other => Err(Error::InvalidFormat(format!("unknown surface: {other}"))),
        }
    }
}
