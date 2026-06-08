//! paddock 内部 RaceId と netkeiba 12 桁 race_id の相互変換。
//!
//! - paddock RaceId: `{year}-{round}-{venue_slug}-{day}-{race_num}R`
//!   （例 `2026-3-tokyo-2-11R`）。出馬表 PDF 取り込み（entry-parser）と同形式。
//! - netkeiba 12 桁: `YYYY` + 場コード 2 桁 + 回 2 桁 + 日 2 桁 + R 2 桁
//!   （例 `202605030211`）。場コードは `Venue::as_code`（01 札幌〜10 小倉）。

use paddock_domain::{RaceId, Venue};

use crate::error::{Error, Result};

/// レース構成要素から netkeiba 12 桁 race_id と paddock RaceId の両方を組み立てる。
///
/// netkeiba 側は回/日/R を 2 桁ゼロ詰めにする。paddock 側は entry-parser と揃え、
/// 回/日/R はゼロ詰めしない素の数値で表記する。
pub fn build_race_ids(
    year: u32,
    venue: Venue,
    round: u32,
    day: u32,
    race_num: u32,
) -> Result<(String, RaceId)> {
    let netkeiba = format!(
        "{:04}{}{:02}{:02}{:02}",
        year,
        venue.as_code(),
        round,
        day,
        race_num
    );
    let paddock_str = format!("{year}-{round}-{}-{day}-{race_num}R", venue.as_slug());
    let race_id = RaceId::try_from(paddock_str)?;
    Ok((netkeiba, race_id))
}

/// netkeiba 12 桁 race_id を paddock RaceId に変換する。
///
/// 12 桁でない、JRA 外（場コードが 01〜10 以外）の ID は `InvalidArgument`。
pub fn paddock_race_id_from_netkeiba(netkeiba_race_id: &str) -> Result<RaceId> {
    let (year, venue, round, day, race_num) = parse_netkeiba_race_id(netkeiba_race_id)?;
    let (_, race_id) = build_race_ids(year, venue, round, day, race_num)?;
    Ok(race_id)
}

/// netkeiba 12 桁 race_id を構成要素 `(year, venue, round, day, race_num)` に分解する。
fn parse_netkeiba_race_id(id: &str) -> Result<(u32, Venue, u32, u32, u32)> {
    if id.len() != 12 || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::InvalidArgument(format!(
            "netkeiba race_id は 12 桁の数字である必要があります: {id}"
        )));
    }
    let year: u32 = id[0..4]
        .parse()
        .map_err(|_| Error::InvalidArgument(format!("invalid year in race_id: {id}")))?;
    let venue = venue_from_code(&id[4..6])
        .ok_or_else(|| Error::InvalidArgument(format!("JRA 外の場コードです: {id}")))?;
    let round: u32 = id[6..8]
        .parse()
        .map_err(|_| Error::InvalidArgument(format!("invalid round in race_id: {id}")))?;
    let day: u32 = id[8..10]
        .parse()
        .map_err(|_| Error::InvalidArgument(format!("invalid day in race_id: {id}")))?;
    let race_num: u32 = id[10..12]
        .parse()
        .map_err(|_| Error::InvalidArgument(format!("invalid race_num in race_id: {id}")))?;
    Ok((year, venue, round, day, race_num))
}

fn venue_from_code(code: &str) -> Option<Venue> {
    let venue = match code {
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
    };
    Some(venue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_both_ids_from_parts() {
        // 2026 年 3 回 東京 2 日 11R → netkeiba 202605030211 / paddock 2026-3-tokyo-2-11R。
        let (netkeiba, race_id) = build_race_ids(2026, Venue::Tokyo, 3, 2, 11).unwrap();
        assert_eq!(netkeiba, "202605030211");
        assert_eq!(race_id.value(), "2026-3-tokyo-2-11R");
    }

    #[test]
    fn zero_pads_netkeiba_but_not_paddock() {
        // 1 回 札幌 1 日 1R。netkeiba はゼロ詰め、paddock は素の数値。
        let (netkeiba, race_id) = build_race_ids(2026, Venue::Sapporo, 1, 1, 1).unwrap();
        assert_eq!(netkeiba, "202601010101");
        assert_eq!(race_id.value(), "2026-1-sapporo-1-1R");
    }

    #[test]
    fn converts_netkeiba_to_paddock() {
        let race_id = paddock_race_id_from_netkeiba("202605030211").unwrap();
        assert_eq!(race_id.value(), "2026-3-tokyo-2-11R");
    }

    #[test]
    fn rejects_non_jra_and_malformed() {
        // 地方（場コード 44）・桁不足・非数字は弾く。
        assert!(paddock_race_id_from_netkeiba("202644010101").is_err());
        assert!(paddock_race_id_from_netkeiba("2026").is_err());
        assert!(paddock_race_id_from_netkeiba("20260503021X").is_err());
    }

    #[test]
    fn roundtrips_through_netkeiba() {
        let (netkeiba, expected) = build_race_ids(2026, Venue::Hanshin, 2, 4, 8).unwrap();
        let back = paddock_race_id_from_netkeiba(&netkeiba).unwrap();
        assert_eq!(back.value(), expected.value());
    }
}
