use strum_macros::Display;

use crate::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum Weather {
    #[strum(to_string = "晴")]
    Sunny,
    #[strum(to_string = "曇")]
    Cloudy,
    #[strum(to_string = "雨")]
    Rainy,
    #[strum(to_string = "小雨")]
    LightRain,
    #[strum(to_string = "雪")]
    Snowy,
    #[strum(to_string = "小雪")]
    LightSnow,
}

impl Weather {
    pub fn as_str(&self) -> &'static str {
        match self {
            Weather::Sunny => "晴",
            Weather::Cloudy => "曇",
            Weather::Rainy => "雨",
            Weather::LightRain => "小雨",
            Weather::Snowy => "雪",
            Weather::LightSnow => "小雪",
        }
    }
}

impl TryFrom<&str> for Weather {
    type Error = Error;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.trim() {
            "晴" => Ok(Weather::Sunny),
            "曇" => Ok(Weather::Cloudy),
            "雨" => Ok(Weather::Rainy),
            "小雨" => Ok(Weather::LightRain),
            "雪" => Ok(Weather::Snowy),
            "小雪" => Ok(Weather::LightSnow),
            other => Err(Error::InvalidFormat(format!("unknown weather: {other}"))),
        }
    }
}
