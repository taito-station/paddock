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
use std::time::Duration;

use paddock_domain::{BetType, RaceId, RaceOdds};
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
    pub wide: Option<String>,
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
    if let Some(html) = &pages.wide {
        odds.wide = parse::parse_wide(html)?;
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
    /// Per-request pacing: slept before each `post_cname`. JRA への礼節のため、
    /// 監視（#257）のように同一オッズを繰り返し叩く用途で間隔を空ける。既定はゼロ。
    delay: Duration,
}

impl Default for UreqOddsScraper {
    fn default() -> Self {
        Self {
            endpoint: ODDS_MENU_URL.to_string(),
            delay: Duration::ZERO,
        }
    }
}

impl UreqOddsScraper {
    pub fn new() -> Self {
        Self::default()
    }

    /// 各リクエスト前に `delay` だけ待つスクレイパを作る（#257 の発走直前監視で
    /// JRA を繰り返し叩くためのペーシング。`UreqNetkeibaScraper::with_delay` と同パターン）。
    pub fn with_delay(delay: Duration) -> Self {
        Self {
            delay,
            ..Self::default()
        }
    }

    /// POST a `cname` token to the odds endpoint and return the page body.
    fn post_cname(&self, cname: &str) -> Result<String> {
        // 1 レースあたり券種ごとに複数 POST するため、リクエスト単位で間隔を空ける。
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        let resp = ureq::post(&self.endpoint)
            .send_form([("cname", cname)])
            .map_err(|e| Error::Fetch(e.to_string()))?;
        // JRA は本文を EUC-JP で返すため、UTF-8 前提の read_to_string では
        // 「stream did not contain valid UTF-8」で失敗する。生バイトで受けてから
        // EUC-JP デコードする（netkeiba-scraper と共通の scraper_util を使う）。
        let mut bytes = Vec::new();
        resp.into_body()
            .into_reader()
            .read_to_end(&mut bytes)
            .map_err(|e| Error::Fetch(format!("read odds body (cname={cname}): {e}")))?;
        Ok(scraper_util::decode_euc_jp(&bytes, cname))
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
        // doAction('/JRADB/accessO.html', '<cname>') — splitting the remainder
        // on the quote char yields ["", ", ", "<cname>", ")"], so the token is
        // the 3rd segment (index 2), not the separator at index 1.
        let Some(cname) = onclick
            .split_once("accessO.html")
            .and_then(|(_, rest)| rest.split('\'').nth(2))
        else {
            continue;
        };
        let label = link.text().collect::<String>().trim().to_string();
        tokens.push((label, cname.to_string()));
    }
    tokens
}

/// Normalise a JRA bet-type label for matching: kanji `三` and any full-width
/// digits used in the `三連*` / `３連*` labels are folded to ASCII digits, so a
/// single comparison covers JRA's mixed notations.
fn normalize_label(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '三' => '3',
            '０'..='９' => char::from(b'0' + (c as u32 - '０' as u32) as u8),
            other => other,
        })
        .collect()
}

/// Resolve the cname navigation token for a bet type from the menu tokens,
/// matching on the (numeral-folded) Japanese label. Pure: unit-testable.
fn match_token(tokens: &[(String, String)], bet: BetType) -> Option<String> {
    let target = normalize_label(bet.as_ja());
    tokens
        .iter()
        .find(|(label, _)| normalize_label(label).contains(&target))
        .map(|(_, cname)| cname.clone())
}

impl OddsScraper for UreqOddsScraper {
    fn scrape(&self, race_id: &RaceId) -> UcResult<RaceOdds> {
        // NOTE: JRA has no race-id → odds URL mapping. This best-effort live
        // path treats the RaceId value as the odds-menu navigation token,
        // fetches the menu, then follows each bet type's cname link. The token
        // convention is unverified against a live race day (see ADR 0001).
        tracing::debug!(race_id = %race_id, "scraping JRA odds");
        let menu = self.post_cname(race_id.value())?;
        let tokens = extract_cname_tokens(&menu);

        // Fail loudly rather than returning empty odds: no tokens means the
        // menu navigation did not resolve (wrong entry token / layout change),
        // which is a different condition from an open race with empty pools.
        if tokens.is_empty() {
            return Err(Error::Fetch(format!(
                "no odds-menu navigation tokens found for race {race_id}; \
                 live JRA navigation is unverified (see ADR 0001)"
            ))
            .into());
        }

        let fetch = |cname: Option<String>| -> Result<Option<String>> {
            match cname {
                Some(c) => Ok(Some(self.post_cname(&c)?)),
                None => Ok(None),
            }
        };

        let pages = OddsPages {
            win_place: fetch(match_token(&tokens, BetType::Win))?,
            quinella: fetch(match_token(&tokens, BetType::Quinella))?,
            wide: fetch(match_token(&tokens, BetType::Wide))?,
            exacta: fetch(match_token(&tokens, BetType::Exacta))?,
            trio: fetch(match_token(&tokens, BetType::Trio))?,
            trifecta: fetch(match_token(&tokens, BetType::Trifecta))?,
        };

        Ok(assemble(race_id.clone(), &pages)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Representative odds-menu markup: each bet type is a doAction link whose
    // second argument is the cname navigation token.
    const MENU: &str = r##"
        <ul>
          <li><a onclick="return doAction('/JRADB/accessO.html', 'pwTAN001')">単勝・複勝</a></li>
          <li><a onclick="return doAction('/JRADB/accessO.html', 'pwUMR002')">馬連</a></li>
          <li><a onclick="return doAction('/JRADB/accessO.html', 'pwWID006')">ワイド</a></li>
          <li><a onclick="return doAction('/JRADB/accessO.html','pwUMT003')">馬単</a></li>
          <li><a onclick="return doAction('/JRADB/accessO.html', 'pwSF004')">3連複</a></li>
          <li><a onclick="return doAction('/JRADB/accessO.html', 'pwST005')">3連単</a></li>
          <li><a href="#">オッズトップ</a></li>
        </ul>
    "##;

    #[test]
    fn extracts_cname_token_not_the_separator() {
        let tokens = extract_cname_tokens(MENU);
        // The bug we guard against: returning the "," separator instead of token.
        assert!(tokens.iter().all(|(_, cname)| cname.starts_with("pw")));
        let win = tokens
            .iter()
            .find(|(label, _)| label.contains("単勝"))
            .map(|(_, c)| c.as_str());
        assert_eq!(win, Some("pwTAN001"));
    }

    #[test]
    fn skips_links_without_doaction() {
        let tokens = extract_cname_tokens(MENU);
        // The plain "オッズトップ" anchor has no onclick and is excluded.
        assert_eq!(tokens.len(), 6);
    }

    #[test]
    fn normalize_label_folds_kanji_and_fullwidth_numerals() {
        assert_eq!(normalize_label("三連複"), "3連複");
        assert_eq!(normalize_label("３連単"), "3連単"); // full-width digit
        assert_eq!(normalize_label("3連単"), "3連単"); // already ASCII
    }

    #[test]
    fn match_token_resolves_each_bet_type_label() {
        let tokens = extract_cname_tokens(MENU);
        // 単勝/複勝 share the "単勝・複勝" menu entry.
        assert_eq!(
            match_token(&tokens, BetType::Win).as_deref(),
            Some("pwTAN001")
        );
        assert_eq!(
            match_token(&tokens, BetType::Quinella).as_deref(),
            Some("pwUMR002")
        );
        assert_eq!(
            match_token(&tokens, BetType::Wide).as_deref(),
            Some("pwWID006")
        );
        // 三連複 (as_ja) matches the menu's "3連複" via numeral folding.
        assert_eq!(
            match_token(&tokens, BetType::Trio).as_deref(),
            Some("pwSF004")
        );
        assert_eq!(
            match_token(&tokens, BetType::Trifecta).as_deref(),
            Some("pwST005")
        );
    }
}
