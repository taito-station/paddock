use paddock_domain::Venue;

/// Upper bounds for meeting-day enumeration, set a little above the real JRA
/// maxima (~6 rounds / ~12 days per meeting). The sequential range fetch uses
/// them as backstops for its 404/403-boundary discovery; the parallel fetch
/// enumerates the full grid up to these caps, where absent days resolve to
/// 403/404 and are counted as not-found.
pub(crate) const ROUND_CAP: u32 = 8;
pub(crate) const DAY_CAP: u32 = 14;

/// A single JRA meeting day, identifying exactly one result PDF.
///
/// JRA publishes one result PDF per (year, round, venue, day), e.g.
/// `2026-3nakayama6.pdf` = 2026 / 3rd meeting / Nakayama / day 6.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingSpec {
    pub year: i32,
    pub round: u32,
    pub venue: Venue,
    pub day: u32,
}

impl MeetingSpec {
    /// Stable key used for the fetch-history table (exclusive control).
    /// e.g. `2026-3-nakayama-6`.
    pub fn source_key(&self) -> String {
        format!(
            "{}-{}-{}-{}",
            self.year,
            self.round,
            self.venue.as_slug(),
            self.day
        )
    }

    /// The JRA result-PDF URL for this meeting day.
    /// e.g. `https://www.jra.go.jp/datafile/seiseki/report/2026/2026-3nakayama6.pdf`.
    pub fn pdf_url(&self) -> String {
        format!(
            "https://www.jra.go.jp/datafile/seiseki/report/{year}/{filename}",
            year = self.year,
            filename = self.pdf_filename(),
        )
    }

    /// The JRA result-PDF file name for this meeting day (the last URL segment).
    /// e.g. `2026-3nakayama6.pdf`. Used as the inbox file name in Stage1 so that
    /// Stage2 can recover the [`MeetingSpec`] from the file via [`Self::from_pdf_filename`].
    pub fn pdf_filename(&self) -> String {
        format!(
            "{year}-{round}{venue}{day}.pdf",
            year = self.year,
            round = self.round,
            venue = self.venue.as_slug(),
            day = self.day,
        )
    }

    /// Recover a [`MeetingSpec`] from a result-PDF file name (or its stem), the
    /// inverse of [`Self::pdf_filename`]. e.g. `2026-3nakayama6.pdf` →
    /// `{2026, round 3, Nakayama, day 6}`. Returns `None` if the name does not
    /// match the `{year}-{round}{venue-slug}{day}` shape (so non-meeting files
    /// are left untouched by Stage2). The slug is the lowercase romaji venue
    /// name; round/day are the surrounding digit runs.
    pub fn from_pdf_filename(name: &str) -> Option<Self> {
        let stem = name.strip_suffix(".pdf").unwrap_or(name);
        let (year_str, rest) = stem.split_once('-')?;
        let year: i32 = year_str.parse().ok()?;

        // rest = "{round}{venue-slug}{day}", e.g. "3nakayama6".
        let round_len = rest.find(|c: char| !c.is_ascii_digit())?;
        let day_len = rest.rfind(|c: char| !c.is_ascii_digit())?;
        // round = leading digits, day = trailing digits, venue = the alphabetic middle.
        if round_len == 0 || day_len + 1 >= rest.len() {
            return None;
        }
        let round: u32 = rest[..round_len].parse().ok()?;
        let venue_slug = &rest[round_len..=day_len];
        let day: u32 = rest[day_len + 1..].parse().ok()?;
        let venue = Venue::try_from(venue_slug).ok()?;

        Some(MeetingSpec {
            year,
            round,
            venue,
            day,
        })
    }
}

/// Result of a meeting-day fetch attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchMeetingOutcome {
    /// Fetched and ingested; carries saved counts.
    Ingested {
        races_saved: usize,
        horses_saved: usize,
    },
    /// Stage1 (`--download-only`): fetched and written to inbox, **not** parsed.
    /// Carries the inbox path the PDF was written to.
    Downloaded { path: std::path::PathBuf },
    /// Already in fetch history; skipped without fetching. In Stage1 this also
    /// covers a meeting already downloaded (inbox) but not yet ingested.
    Skipped,
    /// The PDF does not exist (HTTP 403 or 404) — e.g. not published yet, or past
    /// the meeting's real round/day range. Carries the HTTP status so the range
    /// loop can persist a boundary absence as a retryable `failed` row (#170).
    NotFound { http_status: u16 },
    /// The PDF was fetched but parsed to **zero races** (e.g. a parser gap for a
    /// particular PDF generation). Deliberately *not* recorded in fetch history,
    /// so the meeting stays a re-fetch candidate instead of being silently
    /// self-blocked as a "successful" 0-race ingest. See issue #149.
    Empty,
}

#[derive(Debug, Clone)]
pub struct FetchMeetingResponse {
    pub source_key: String,
    pub url: String,
    pub outcome: FetchMeetingOutcome,
}

/// A range of JRA meeting days to fetch. Omitting a field widens the range:
/// no `day` = every day of the round, no `round` = every round of the venue,
/// no `venue` = every venue in the year. `year` is always required.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingRange {
    pub year: i32,
    pub venue: Option<Venue>,
    pub round: Option<u32>,
    pub day: Option<u32>,
}

impl MeetingRange {
    /// Every meeting-day candidate covered by this range, for the parallel fetch.
    ///
    /// A field that is `Some` pins that axis; a `None` field is expanded over its
    /// full domain (`Venue::all()` for venue, `1..=ROUND_CAP` / `1..=DAY_CAP` for
    /// round / day). The caps comfortably exceed real JRA maxima, so non-existent
    /// combinations are included and simply resolve to 403/404 (counted as not-found)
    /// rather than being pruned. The grid can be large — up to
    /// `venues × ROUND_CAP × DAY_CAP` (≈1120 for a whole year) — but the extra
    /// probes are cheap GETs, traded for dropping the sequential boundary bookkeeping.
    pub fn candidate_specs(&self) -> Vec<MeetingSpec> {
        let venues: Vec<Venue> = match self.venue {
            Some(v) => vec![v],
            None => Venue::all().to_vec(),
        };
        let rounds: Vec<u32> = match self.round {
            Some(r) => vec![r],
            None => (1..=ROUND_CAP).collect(),
        };
        let days: Vec<u32> = match self.day {
            Some(d) => vec![d],
            None => (1..=DAY_CAP).collect(),
        };

        let mut specs = Vec::with_capacity(venues.len() * rounds.len() * days.len());
        for &venue in &venues {
            for &round in &rounds {
                for &day in &days {
                    specs.push(MeetingSpec {
                        year: self.year,
                        round,
                        venue,
                        day,
                    });
                }
            }
        }
        specs
    }
}

/// Aggregate result of a range fetch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FetchRangeSummary {
    pub ingested: usize,
    /// Stage1 (`--download-only`): meetings fetched to inbox without parsing.
    pub downloaded: usize,
    pub skipped: usize,
    pub not_found: usize,
    pub failed: usize,
    /// Boundary 403/404s (absent day right after ≥1 existing day in the same round)
    /// persisted as retryable `failed` rows by the sequential range fetch (#170).
    /// A subset of `not_found`: counts only the absences recorded for re-try, not
    /// the round-nonexistence boundaries (day-1 absent) or parallel-grid junk.
    pub recorded_failed: usize,
    /// Meetings whose PDF was fetched but parsed to zero races (not recorded,
    /// so still re-fetchable). Tracked separately from `failed` because the PDF
    /// exists — it is a parser gap, not a fetch error.
    pub empty: usize,
    pub races_saved: usize,
    pub horses_saved: usize,
    /// (source_key, error message) for each meeting that errored mid-range.
    pub failures: Vec<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_filename_roundtrips_through_from_pdf_filename() {
        let spec = MeetingSpec {
            year: 2026,
            round: 3,
            venue: Venue::Nakayama,
            day: 6,
        };
        assert_eq!(spec.pdf_filename(), "2026-3nakayama6.pdf");
        assert_eq!(
            MeetingSpec::from_pdf_filename("2026-3nakayama6.pdf"),
            Some(spec.clone())
        );
        // The stem (no extension) is accepted too.
        assert_eq!(
            MeetingSpec::from_pdf_filename("2026-3nakayama6"),
            Some(spec)
        );
        // Multi-digit round/day still split on the alphabetic venue.
        assert_eq!(
            MeetingSpec::from_pdf_filename("2026-10tokyo12"),
            Some(MeetingSpec {
                year: 2026,
                round: 10,
                venue: Venue::Tokyo,
                day: 12,
            })
        );
    }

    #[test]
    fn from_pdf_filename_rejects_non_meeting_names() {
        assert_eq!(MeetingSpec::from_pdf_filename("notes.pdf"), None); // no year-dash
        assert_eq!(MeetingSpec::from_pdf_filename("2026-nakayama6"), None); // missing round
        assert_eq!(MeetingSpec::from_pdf_filename("2026-3nakayama"), None); // missing day
        assert_eq!(MeetingSpec::from_pdf_filename("2026-3mars6"), None); // unknown venue slug
        assert_eq!(MeetingSpec::from_pdf_filename("2026-12345"), None); // digits only, no venue slug
        assert_eq!(MeetingSpec::from_pdf_filename("-3nakayama6"), None); // empty year
    }

    #[test]
    fn candidate_specs_pins_fixed_axes() {
        let range = MeetingRange {
            year: 2025,
            venue: Some(Venue::Nakayama),
            round: Some(3),
            day: Some(6),
        };
        assert_eq!(
            range.candidate_specs(),
            vec![MeetingSpec {
                year: 2025,
                round: 3,
                venue: Venue::Nakayama,
                day: 6,
            }]
        );
    }

    #[test]
    fn candidate_specs_expands_open_axes_to_caps() {
        // venue fixed, round/day open → 1 venue * ROUND_CAP * DAY_CAP
        let range = MeetingRange {
            year: 2025,
            venue: Some(Venue::Tokyo),
            round: None,
            day: None,
        };
        let specs = range.candidate_specs();
        assert_eq!(specs.len() as u32, ROUND_CAP * DAY_CAP);
        assert!(
            specs
                .iter()
                .all(|s| s.venue == Venue::Tokyo && s.year == 2025)
        );
        assert!(
            specs
                .iter()
                .any(|s| s.round == ROUND_CAP && s.day == DAY_CAP)
        );
    }

    #[test]
    fn candidate_specs_whole_year_covers_every_venue() {
        let range = MeetingRange {
            year: 2025,
            venue: None,
            round: None,
            day: None,
        };
        let specs = range.candidate_specs();
        assert_eq!(
            specs.len() as u32,
            Venue::all().len() as u32 * ROUND_CAP * DAY_CAP
        );
        for venue in Venue::all() {
            assert!(specs.iter().any(|s| s.venue == venue));
        }
    }
}
