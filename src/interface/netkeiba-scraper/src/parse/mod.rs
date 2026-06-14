//! 純粋なHTMLパース層（ネットワーク無し）。出馬表と馬個別成績を fixture で網羅テストする。

mod card;
mod horse_history;
mod odds;
mod payout;
mod result;
mod shutuba;

pub use card::parse_card;
pub use horse_history::parse_horse_history;
pub use odds::{
    parse_exacta_odds, parse_quinella_odds, parse_trifecta_odds, parse_trio_odds,
    parse_win_place_odds,
};
pub use payout::parse_race_payouts;
pub use result::parse_race_result;
pub use shutuba::parse_shutuba;

use paddock_domain::Venue;

/// netkeiba の 12 桁 race_id から JRA 場コード(5〜6 桁目)を Venue に変換する。
/// JRA は 01〜10。地方(30 番台〜)・海外はここで `None` となり、呼び出し側で行スキップ。
pub(crate) fn venue_from_race_id(race_id: &str) -> Option<Venue> {
    Venue::from_code(race_id.get(4..6)?)
}

/// race_id の 7〜8 桁目=開催回, 9〜10 桁目=開催日次, 11〜12 桁目=R を u32 で取り出す。
pub(crate) fn round_day_racenum(race_id: &str) -> Option<(u32, u32, u32)> {
    let round = race_id.get(6..8)?.parse().ok()?;
    let day = race_id.get(8..10)?.parse().ok()?;
    let race_num = race_id.get(10..12)?.parse().ok()?;
    Some((round, day, race_num))
}

/// セルのテキストを取り出し、前後空白と `&nbsp;`(U+00A0) を除いて返す。空なら `None`。
pub(crate) fn cell_text(s: &str) -> Option<String> {
    let t = s.replace('\u{a0}', " ");
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() { None } else { Some(t) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn venue_maps_jra_codes_and_rejects_others() {
        // JRA10場（先頭は year4桁、5〜6桁目が場コード）。
        assert_eq!(venue_from_race_id("202401020411"), Some(Venue::Sapporo)); // 01
        assert_eq!(venue_from_race_id("202405030211"), Some(Venue::Tokyo)); // 05
        assert_eq!(venue_from_race_id("202409010101"), Some(Venue::Hanshin)); // 09
        assert_eq!(venue_from_race_id("202410010101"), Some(Venue::Kokura)); // 10
        // 地方（大井=44 等 JRA 外コード）・短すぎる ID は None でスキップ。
        assert_eq!(venue_from_race_id("202444010101"), None);
        assert_eq!(venue_from_race_id("2024"), None);
    }

    #[test]
    fn round_day_racenum_splits_id() {
        // 2024 05 03 02 11 → round=3, day=2, race=11
        assert_eq!(round_day_racenum("202405030211"), Some((3, 2, 11)));
    }
}
