use strum_macros::Display;

use crate::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum Venue {
    #[strum(to_string = "札幌")]
    Sapporo,
    #[strum(to_string = "函館")]
    Hakodate,
    #[strum(to_string = "福島")]
    Fukushima,
    #[strum(to_string = "新潟")]
    Niigata,
    #[strum(to_string = "東京")]
    Tokyo,
    #[strum(to_string = "中山")]
    Nakayama,
    #[strum(to_string = "中京")]
    Chukyo,
    #[strum(to_string = "京都")]
    Kyoto,
    #[strum(to_string = "阪神")]
    Hanshin,
    #[strum(to_string = "小倉")]
    Kokura,
}

impl Venue {
    pub fn as_jp(&self) -> &'static str {
        match self {
            Venue::Sapporo => "札幌",
            Venue::Hakodate => "函館",
            Venue::Fukushima => "福島",
            Venue::Niigata => "新潟",
            Venue::Tokyo => "東京",
            Venue::Nakayama => "中山",
            Venue::Chukyo => "中京",
            Venue::Kyoto => "京都",
            Venue::Hanshin => "阪神",
            Venue::Kokura => "小倉",
        }
    }

    pub fn as_slug(&self) -> &'static str {
        match self {
            Venue::Sapporo => "sapporo",
            Venue::Hakodate => "hakodate",
            Venue::Fukushima => "fukushima",
            Venue::Niigata => "niigata",
            Venue::Tokyo => "tokyo",
            Venue::Nakayama => "nakayama",
            Venue::Chukyo => "chukyo",
            Venue::Kyoto => "kyoto",
            Venue::Hanshin => "hanshin",
            Venue::Kokura => "kokura",
        }
    }
}

impl TryFrom<&str> for Venue {
    type Error = Error;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.trim() {
            "札幌" | "sapporo" => Ok(Venue::Sapporo),
            "函館" | "hakodate" => Ok(Venue::Hakodate),
            "福島" | "fukushima" => Ok(Venue::Fukushima),
            "新潟" | "niigata" => Ok(Venue::Niigata),
            "東京" | "tokyo" => Ok(Venue::Tokyo),
            "中山" | "nakayama" => Ok(Venue::Nakayama),
            "中京" | "chukyo" => Ok(Venue::Chukyo),
            "京都" | "kyoto" => Ok(Venue::Kyoto),
            "阪神" | "hanshin" => Ok(Venue::Hanshin),
            "小倉" | "kokura" => Ok(Venue::Kokura),
            other => Err(Error::InvalidFormat(format!("unknown venue: {other}"))),
        }
    }
}
