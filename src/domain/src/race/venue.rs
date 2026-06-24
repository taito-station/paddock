use strum_macros::Display;

use crate::error::Error;

// Hash は backtest の course_stats キャッシュキー (Venue, u32, Surface) に必要（#223）。
// Surface も元から Hash を derive 済み。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display)]
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
    /// All 10 JRA venues, in the conventional course-code order. Used to enumerate
    /// every venue when a range fetch omits `--venue`.
    pub fn all() -> [Venue; 10] {
        [
            Venue::Sapporo,
            Venue::Hakodate,
            Venue::Fukushima,
            Venue::Niigata,
            Venue::Tokyo,
            Venue::Nakayama,
            Venue::Chukyo,
            Venue::Kyoto,
            Venue::Hanshin,
            Venue::Kokura,
        ]
    }

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

    /// JRA 場コード（"01".."10"）を返す。netkeiba 12 桁 race_id の 5〜6 桁目に対応し、
    /// `parse::venue_from_race_id` の逆変換にあたる（netkeiba race_id 組み立て用）。
    pub fn as_code(&self) -> &'static str {
        match self {
            Venue::Sapporo => "01",
            Venue::Hakodate => "02",
            Venue::Fukushima => "03",
            Venue::Niigata => "04",
            Venue::Tokyo => "05",
            Venue::Nakayama => "06",
            Venue::Chukyo => "07",
            Venue::Kyoto => "08",
            Venue::Hanshin => "09",
            Venue::Kokura => "10",
        }
    }

    /// JRA 場コード（"01".."10"）から Venue を引く（[`Venue::as_code`] の逆）。
    /// JRA 外（地方=30番台〜・海外）のコードは `None`。netkeiba 12 桁 race_id の
    /// 5〜6 桁目の解釈に用いる、場コード↔Venue 対応の単一の正本。
    pub fn from_code(code: &str) -> Option<Venue> {
        Some(match code {
            "01" => Venue::Sapporo,
            "02" => Venue::Hakodate,
            "03" => Venue::Fukushima,
            "04" => Venue::Niigata,
            "05" => Venue::Tokyo,
            "06" => Venue::Nakayama,
            "07" => Venue::Chukyo,
            "08" => Venue::Kyoto,
            "09" => Venue::Hanshin,
            "10" => Venue::Kokura,
            _ => return None,
        })
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
