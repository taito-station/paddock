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

    /// 洋芝（北海道開催＝札幌・函館）の場か。両場は同じ洋芝でコース適性が通じるため、
    /// 条件別実績（#628・提示専用）では `turf_group` で 1 グループとして集計できる。
    /// **確率推定には入れない**——純モデルの resolution 天井は ADR 0058/0059 で決着済み。
    pub fn is_yoshiba(&self) -> bool {
        matches!(self, Venue::Sapporo | Venue::Hakodate)
    }

    /// 芝の適性が通じる場グループ（自身を必ず含む）。洋芝場は札幌・函館の 2 場、
    /// それ以外の場は自身 1 場のみ（＝グループ化しても完全一致と同じ集合になる）。
    pub fn turf_group(&self) -> &'static [Venue] {
        const YOSHIBA: &[Venue] = &[Venue::Sapporo, Venue::Hakodate];
        match self {
            Venue::Sapporo | Venue::Hakodate => YOSHIBA,
            Venue::Fukushima => &[Venue::Fukushima],
            Venue::Niigata => &[Venue::Niigata],
            Venue::Tokyo => &[Venue::Tokyo],
            Venue::Nakayama => &[Venue::Nakayama],
            Venue::Chukyo => &[Venue::Chukyo],
            Venue::Kyoto => &[Venue::Kyoto],
            Venue::Hanshin => &[Venue::Hanshin],
            Venue::Kokura => &[Venue::Kokura],
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yoshiba_venues_are_sapporo_and_hakodate() {
        for v in Venue::all() {
            assert_eq!(
                v.is_yoshiba(),
                matches!(v, Venue::Sapporo | Venue::Hakodate),
                "{v:?} の洋芝判定が想定と違う"
            );
        }
    }

    #[test]
    fn turf_group_pairs_yoshiba_and_isolates_the_rest() {
        // 洋芝場は互いを含む 2 場グループ。
        assert_eq!(
            Venue::Sapporo.turf_group(),
            &[Venue::Sapporo, Venue::Hakodate]
        );
        assert_eq!(
            Venue::Hakodate.turf_group(),
            &[Venue::Sapporo, Venue::Hakodate]
        );
        // 洋芝以外は自身のみ＝グループ化しても完全一致と同じ集合になる。
        for v in Venue::all().into_iter().filter(|v| !v.is_yoshiba()) {
            assert_eq!(v.turf_group(), &[v], "{v:?} が単独グループになっていない");
        }
    }

    #[test]
    fn turf_group_always_contains_self() {
        for v in Venue::all() {
            assert!(
                v.turf_group().contains(&v),
                "{v:?} の turf_group が自身を含んでいない"
            );
        }
    }
}
