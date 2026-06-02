//! Live JRA odds navigation (best-effort) plus the pure assembly step.
//!
//! ## Why navigation is non-trivial
//!
//! JRA odds pages are not addressable by a stable GET URL. The odds menu links
//! are JavaScript `doAction('/JRADB/accessO.html', '<cname>')` calls, where
//! `<cname>` is a per-page session token. Reaching a bet type's odds therefore
//! means: GET the odds menu → scrape the `cname` token for that bet type →
//! POST it back to `accessO.html`. There is no race-id query parameter.
//!
//! The networking here is **best-effort and validated only at the parsing
//! layer** (see [`crate::parse`] and the fixture tests): the token mechanism
//! can shift and only a live race day exercises it end to end. [`assemble`] is
//! the pure, fully-tested core that turns fetched HTML into [`RaceOdds`].

use std::io::Read;

use paddock_domain::{RaceId, RaceOdds};
use paddock_use_case::Result as UcResult;
use paddock_use_case::odds_scraper::OddsScraper;
use scraper::{Html, Selector};

use crate::error::{Error, Result};
use crate::parse;

/// Raw odds HTML for one race, one page per bet type. A page left `None` simply
/// contributes no odds to the assembled [`RaceOdds`].
#[derive(Debug, Default, Clone)]
pub struct OddsPages {
    pub win_place: Option<String>,
    pub quinella: Option<String>,
    pub exacta: Option<String>,
    pub trio: Option<String>,
    pub trifecta: Option<String>,
}

/// Assemble fetched per-bet-type HTML into a [`RaceOdds`]. Pure: no network.
pub fn assemble(race_id: RaceId, pages: &OddsPages) -> Result<RaceOdds> {
    let mut odds = RaceOdds::empty(race_id);

    if let Some(html) = &pages.win_place {
        let (win, place) = parse::parse_win_place(html)?;
        odds.win = win;
        odds.place = place;
    }
    if let Some(html) = &pages.quinella {
        odds.quinella = parse::parse_quinella(html)?;
    }
    if let Some(html) = &pages.exacta {
        odds.exacta = parse::parse_exacta(html)?;
    }
    if let Some(html) = &pages.trio {
        odds.trio = parse::parse_trio(html)?;
    }
    if let Some(html) = &pages.trifecta {
        odds.trifecta = parse::parse_trifecta(html)?;
    }

    Ok(odds)
}

const ODDS_MENU_URL: &str = "https://www.jra.go.jp/JRADB/accessO.html";

/// Live JRA odds scraper over `ureq` (synchronous, matching the rest of the
/// project's HTTP usage).
pub struct UreqOddsScraper {
    /// The `accessO.html` endpoint; overridable for tests/staging.
    endpoint: String,
}

impl Default for UreqOddsScraper {
    fn default() -> Self {
        Self {
            endpoint: ODDS_MENU_URL.to_string(),
        }
    }
}

impl UreqOddsScraper {
    pub fn new() -> Self {
        Self::default()
    }

    /// POST a `cname` token to the odds endpoint and return the page body.
    fn post_cname(&self, cname: &str) -> Result<String> {
        let resp = ureq::post(&self.endpoint)
            .send_form(&[("cname", cname)])
            .map_err(|e| Error::Fetch(e.to_string()))?;
        let mut body = String::new();
        resp.into_reader()
            .read_to_string(&mut body)
            .map_err(Error::Io)?;
        Ok(body)
    }
}

/// Extract `cname` tokens from an odds menu page, keyed by the Japanese bet
/// label that labels each link. Tokens drive [`UreqOddsScraper::post_cname`].
fn extract_cname_tokens(menu_html: &str) -> Vec<(String, String)> {
    let doc = Html::parse_document(menu_html);
    let link_sel = Selector::parse("a[onclick]").expect("static selector is valid");
    let mut tokens = Vec::new();
    for link in doc.select(&link_sel) {
        let Some(onclick) = link.value().attr("onclick") else {
            continue;
        };
        let Some(cname) = onclick
            .split_once("accessO.html")
            .and_then(|(_, rest)| rest.split('\'').nth(1))
        else {
            continue;
        };
        let label = link.text().collect::<String>().trim().to_string();
        tokens.push((label, cname.to_string()));
    }
    tokens
}

impl OddsScraper for UreqOddsScraper {
    fn scrape(&self, race_id: &RaceId) -> UcResult<RaceOdds> {
        // The caller supplies the race's odds-menu token as the RaceId value.
        // Fetch the menu, then follow each bet type's cname link.
        tracing::debug!(race_id = %race_id, "scraping JRA odds");
        let menu = self.post_cname(race_id.value())?;
        let tokens = extract_cname_tokens(&menu);

        let find = |ja: &str| -> Option<String> {
            tokens
                .iter()
                .find(|(label, _)| label.contains(ja))
                .map(|(_, cname)| cname.clone())
        };

        let fetch = |cname: Option<String>| -> Result<Option<String>> {
            match cname {
                Some(c) => Ok(Some(self.post_cname(&c)?)),
                None => Ok(None),
            }
        };

        let pages = OddsPages {
            win_place: fetch(find("単勝"))?,
            quinella: fetch(find("馬連"))?,
            exacta: fetch(find("馬単"))?,
            trio: fetch(find("3連複").or_else(|| find("三連複")))?,
            trifecta: fetch(find("3連単").or_else(|| find("三連単")))?,
        };

        Ok(assemble(race_id.clone(), &pages)?)
    }
}
