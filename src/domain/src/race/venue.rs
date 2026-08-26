use strum_macros::Display;

use crate::error::Error;
use crate::race::Surface;

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

    /// **芝の**適性が通じる場グループ（自身を必ず含む）。洋芝（北海道開催＝札幌・函館）は
    /// 同じ洋芝でコース適性が通じるため 2 場、それ以外の場は自身 1 場のみ
    /// （＝グループ化しても完全一致と同じ集合になる）。
    ///
    /// **芝限定**である点に注意。同じ 2 場でもダートは別物なので、条件別実績（#628・提示専用）
    /// の集計側は芝のときだけこれを使い、ダートでは [`Venue::self_group`] に落とす。
    /// **確率推定には入れない**——純モデルの resolution 天井は ADR 0058/0059 で決着済み。
    pub fn turf_group(&self) -> &'static [Venue] {
        const YOSHIBA: &[Venue] = &[Venue::Sapporo, Venue::Hakodate];
        match self {
            Venue::Sapporo | Venue::Hakodate => YOSHIBA,
            other => other.self_group(),
        }
    }

    /// 条件別実績（#628・提示専用）の母集団になる場グループ。**芝のときだけ**
    /// [`Venue::turf_group`] で広げ、それ以外（ダート）は当場 1 場に閉じる。
    ///
    /// 洋芝グループの根拠は「**芝の**適性が通じる」なので、同じ 2 場でもダートは別物。
    /// gate しないと「洋芝(札幌/函館)ダ1700m」という成立しないラベルになる。
    ///
    /// **この規則の正本はここ 1 か所**。集計（rdb-gateway）と提示（use-case の
    /// `group_venue_slugs`）が別々に同じ判定を書くと、片方だけ変わったときに
    /// 「グループ見出しは出るのに中身が空」のような乖離が起きる（ADR 0064 の second source）。
    pub fn condition_group(&self, surface: Surface) -> &'static [Venue] {
        match surface {
            Surface::Turf => self.turf_group(),
            // `Surface` が増えたらコンパイルエラーで気づけるよう `_` で受けない。
            Surface::Dirt => self.self_group(),
        }
    }

    /// 自身 1 場だけのグループ。グループ化しない条件（ダート等）で [`Venue::turf_group`] と
    /// 同じ型を返すために使う。
    pub fn self_group(&self) -> &'static [Venue] {
        match self {
            Venue::Sapporo => &[Venue::Sapporo],
            Venue::Hakodate => &[Venue::Hakodate],
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
    fn self_group_is_always_exactly_self() {
        for v in Venue::all() {
            assert_eq!(
                v.self_group(),
                &[v],
                "{v:?} の self_group が自身 1 場でない"
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
        for v in Venue::all()
            .into_iter()
            .filter(|v| !matches!(v, Venue::Sapporo | Venue::Hakodate))
        {
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

    #[test]
    fn condition_group_widens_only_for_yoshiba_turf() {
        // 芝の洋芝場だけが 2 場グループ。
        for v in [Venue::Sapporo, Venue::Hakodate] {
            assert_eq!(
                v.condition_group(Surface::Turf),
                &[Venue::Sapporo, Venue::Hakodate],
                "{v:?} の芝グループが広がっていない"
            );
            // 洋芝の根拠は「芝の適性が通じる」なので、同じ 2 場でもダートは当場のみ。
            assert_eq!(
                v.condition_group(Surface::Dirt),
                &[v],
                "{v:?} のダートでグループが広がっている"
            );
        }
    }

    #[test]
    fn condition_group_is_always_self_only_elsewhere() {
        for v in Venue::all()
            .into_iter()
            .filter(|v| !matches!(v, Venue::Sapporo | Venue::Hakodate))
        {
            for s in [Surface::Turf, Surface::Dirt] {
                assert_eq!(
                    v.condition_group(s),
                    &[v],
                    "{v:?} / {s:?} が単独グループになっていない"
                );
            }
        }
    }
}
