//! 純粋なHTMLパース層（ネットワーク無し）。出馬表と馬個別成績を fixture で網羅テストする。

mod horse_history;
mod shutuba;

pub use horse_history::parse_horse_history;
pub use shutuba::parse_shutuba;

use paddock_domain::Venue;

/// netkeiba の 12 桁 race_id から JRA 場コード(5〜6 桁目)を Venue に変換する。
/// JRA は 01〜10。地方(30 番台〜)・海外はここで `None` となり、呼び出し側で行スキップ。
pub(crate) fn venue_from_race_id(race_id: &str) -> Option<Venue> {
    let code = race_id.get(4..6)?;
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
