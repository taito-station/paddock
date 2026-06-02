//! HTML → domain odds parsing.
//!
//! JRA publishes each bet type on its own odds page. The tables share a common
//! shape: a horse-number (組番) cell plus an odds cell. We target the cell
//! classes JRA uses (`td.num` / `td.odds_tan` / `td.odds_fuku` / `td.odds`) and
//! normalise full-width separators so the same logic works across bet types.
//!
//! Odds cells that are not yet published (`---`, blank) or for scratched horses
//! (`取消`) are skipped rather than erroring, so a partially-open pool still
//! yields the rows that do have odds.

use std::collections::HashMap;

use paddock_domain::{
    HorseNum, OddsValue, OrderedPair, OrderedTriple, Pair, PlaceOdds, Triple,
};
use scraper::{ElementRef, Html, Selector};

use crate::error::{Error, Result};

/// Collapse the inner text of an element into a single trimmed string.
fn text_of(el: ElementRef) -> String {
    el.text().collect::<String>().trim().to_string()
}

fn selector(s: &str) -> Selector {
    Selector::parse(s).expect("static selector is valid")
}

/// Parse a single odds figure. Returns `Ok(None)` for unpublished/scratched
/// cells (`---`, empty, `取消`) so the caller can skip them.
fn parse_odds(raw: &str) -> Result<Option<OddsValue>> {
    let cleaned = raw.trim().replace(',', "");
    if cleaned.is_empty() || cleaned.starts_with("---") || cleaned.contains("取消") {
        return Ok(None);
    }
    let value: f64 = cleaned
        .parse()
        .map_err(|e| Error::Parse(format!("invalid odds '{raw}': {e}")))?;
    Ok(Some(OddsValue::try_from(value)?))
}

/// Parse a single horse number cell into a [`HorseNum`].
fn parse_horse_num(raw: &str) -> Result<HorseNum> {
    let n: u32 = raw
        .trim()
        .parse()
        .map_err(|e| Error::Parse(format!("invalid horse number '{raw}': {e}")))?;
    HorseNum::try_from(n).map_err(Error::from)
}

/// Split a combination cell (e.g. `1 - 2`, `1 → 2 → 3`) into horse numbers.
/// Full-width separators and arrows are normalised to ASCII first.
fn parse_combo(raw: &str) -> Result<Vec<HorseNum>> {
    let normalized = raw
        .replace(['－', '―', '−', '⇒'], "-")
        .replace('→', "-")
        .replace('　', " ");
    normalized
        .split('-')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_horse_num)
        .collect()
}

/// Parse the combined 単勝 / 複勝 page into win and place maps.
pub fn parse_win_place(
    html: &str,
) -> Result<(HashMap<HorseNum, OddsValue>, HashMap<HorseNum, PlaceOdds>)> {
    let doc = Html::parse_document(html);
    let row_sel = selector("tr");
    let num_sel = selector("td.num");
    let tan_sel = selector("td.odds_tan");
    let fuku_sel = selector("td.odds_fuku");

    let mut win = HashMap::new();
    let mut place = HashMap::new();

    for row in doc.select(&row_sel) {
        let Some(num_cell) = row.select(&num_sel).next() else {
            continue;
        };
        let horse = parse_horse_num(&text_of(num_cell))?;

        if let Some(tan) = row.select(&tan_sel).next()
            && let Some(odds) = parse_odds(&text_of(tan))?
        {
            win.insert(horse, odds);
        }

        if let Some(fuku) = row.select(&fuku_sel).next()
            && let Some(band) = parse_place_band(&text_of(fuku))?
        {
            place.insert(horse, band);
        }
    }

    Ok((win, place))
}

/// Parse a 複勝 band cell such as `1.5 - 2.0`. A single figure is treated as a
/// degenerate band (`low == high`).
fn parse_place_band(raw: &str) -> Result<Option<PlaceOdds>> {
    let normalized = raw.replace(['－', '―', '−', '〜', '~'], "-");
    let parts: Vec<&str> = normalized.split('-').map(str::trim).collect();
    match parts.as_slice() {
        [single] => match parse_odds(single)? {
            Some(v) => Ok(Some(PlaceOdds::new(v, v)?)),
            None => Ok(None),
        },
        [low, high, ..] => {
            let (Some(low), Some(high)) = (parse_odds(low)?, parse_odds(high)?) else {
                return Ok(None);
            };
            Ok(Some(PlaceOdds::new(low, high)?))
        }
        [] => Ok(None),
    }
}

/// Shared row iteration for the combination pages: each data row has a `td.num`
/// combination cell and a `td.odds` figure.
fn parse_combo_rows<K, F>(html: &str, mut build_key: F) -> Result<HashMap<K, OddsValue>>
where
    K: std::hash::Hash + Eq,
    F: FnMut(&[HorseNum]) -> Result<K>,
{
    let doc = Html::parse_document(html);
    let row_sel = selector("tr");
    let num_sel = selector("td.num");
    let odds_sel = selector("td.odds");

    let mut map = HashMap::new();
    for row in doc.select(&row_sel) {
        let (Some(num_cell), Some(odds_cell)) =
            (row.select(&num_sel).next(), row.select(&odds_sel).next())
        else {
            continue;
        };
        let Some(odds) = parse_odds(&text_of(odds_cell))? else {
            continue;
        };
        let horses = parse_combo(&text_of(num_cell))?;
        map.insert(build_key(&horses)?, odds);
    }
    Ok(map)
}

fn expect_len(horses: &[HorseNum], n: usize, bet: &str) -> Result<()> {
    if horses.len() != n {
        return Err(Error::Parse(format!(
            "{bet} combination must have {n} horses, got {}",
            horses.len()
        )));
    }
    Ok(())
}

/// 馬連
pub fn parse_quinella(html: &str) -> Result<HashMap<Pair, OddsValue>> {
    parse_combo_rows(html, |h| {
        expect_len(h, 2, "quinella")?;
        Pair::new(h[0], h[1]).map_err(Error::from)
    })
}

/// 馬単
pub fn parse_exacta(html: &str) -> Result<HashMap<OrderedPair, OddsValue>> {
    parse_combo_rows(html, |h| {
        expect_len(h, 2, "exacta")?;
        OrderedPair::new(h[0], h[1]).map_err(Error::from)
    })
}

/// 三連複
pub fn parse_trio(html: &str) -> Result<HashMap<Triple, OddsValue>> {
    parse_combo_rows(html, |h| {
        expect_len(h, 3, "trio")?;
        Triple::new(h[0], h[1], h[2]).map_err(Error::from)
    })
}

/// 三連単
pub fn parse_trifecta(html: &str) -> Result<HashMap<OrderedTriple, OddsValue>> {
    parse_combo_rows(html, |h| {
        expect_len(h, 3, "trifecta")?;
        OrderedTriple::new(h[0], h[1], h[2]).map_err(Error::from)
    })
}
