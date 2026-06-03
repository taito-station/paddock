use paddock_domain::Venue;

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
