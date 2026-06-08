use paddock_domain::Venue;

/// Runaway guards for meeting enumeration. Set a little above the real JRA maxima
/// (~6 rounds / ~12 days per meeting) so a legitimate meeting always falls inside.
/// The sequential range fetch uses these as 404-boundary backstops; the parallel
/// fetch enumerates the whole grid up to these caps (absent days simply 404).
pub const ROUND_CAP: u32 = 8;
pub const DAY_CAP: u32 = 14;

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
            "https://www.jra.go.jp/datafile/seiseki/report/{year}/{year}-{round}{venue}{day}.pdf",
            year = self.year,
            round = self.round,
            venue = self.venue.as_slug(),
            day = self.day,
        )
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
    /// Already in fetch history; skipped without fetching.
    Skipped,
    /// The PDF does not exist (HTTP 404) — e.g. not published yet.
    NotFound,
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
    /// combinations are included and simply resolve to 404 (counted as not-found)
    /// rather than being pruned — the parallel path trades a few cheap 404 probes
    /// for dropping the sequential 404-boundary bookkeeping.
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
    pub skipped: usize,
    pub not_found: usize,
    pub failed: usize,
    pub races_saved: usize,
    pub horses_saved: usize,
    /// (source_key, error message) for each meeting that errored mid-range.
    pub failures: Vec<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(specs.len() as u32, 10 * ROUND_CAP * DAY_CAP);
        for venue in Venue::all() {
            assert!(specs.iter().any(|s| s.venue == venue));
        }
    }
}
