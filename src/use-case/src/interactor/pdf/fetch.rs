use std::time::Duration;

use paddock_domain::Venue;

use crate::dto::pdf::fetch::{
    FetchMeetingOutcome, FetchMeetingResponse, FetchRangeSummary, MeetingRange, MeetingSpec,
};
use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{FetchRecord, Repository};

/// Upper bounds used only to guarantee the discovery loop terminates even if every
/// request errors (a 404 normally stops enumeration well before these). JRA meetings
/// never exceed these in practice.
const ROUND_CAP: u32 = 6;
const DAY_CAP: u32 = 12;

impl<R: Repository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// Fetch a single JRA meeting-day result PDF, parse it, and store the
    /// races. The PDF itself is never written to disk — only a fetch-history
    /// row is kept so the same meeting is not re-ingested on a later run.
    ///
    /// When `force` is false and the meeting is already in fetch history, the
    /// fetch is skipped entirely. A non-existent PDF (HTTP 404) reports
    /// [`FetchMeetingOutcome::NotFound`] and is *not* recorded, so it can be
    /// retried once JRA publishes it.
    pub async fn fetch_meeting(
        &self,
        spec: &MeetingSpec,
        force: bool,
    ) -> Result<FetchMeetingResponse> {
        let source_key = spec.source_key();
        let url = spec.pdf_url();

        if !force && self.repository.fetch_history_contains(&source_key).await? {
            return Ok(FetchMeetingResponse {
                source_key,
                url,
                outcome: FetchMeetingOutcome::Skipped,
            });
        }

        let Some(bytes) = self.pdf_fetcher.fetch_if_exists(&url)? else {
            return Ok(FetchMeetingResponse {
                source_key,
                url,
                outcome: FetchMeetingOutcome::NotFound,
            });
        };

        let races = self.pdf_parser.parse(&bytes)?;
        let mut races_saved = 0usize;
        let mut horses_saved = 0usize;
        for race in &races {
            self.repository.save_race(race).await?;
            races_saved += 1;
            horses_saved += race.results.len();
        }

        self.repository
            .record_fetch(&FetchRecord {
                source_key: source_key.clone(),
                url: url.clone(),
                races_saved: races_saved as u32,
                horses_saved: horses_saved as u32,
                fetched_at: chrono::Utc::now(),
            })
            .await?;

        Ok(FetchMeetingResponse {
            source_key,
            url,
            outcome: FetchMeetingOutcome::Ingested {
                races_saved,
                horses_saved,
            },
        })
    }

    /// Fetch every meeting day in `range`, reusing [`Self::fetch_meeting`] per candidate.
    ///
    /// Discovery is sequential and uses the per-meeting outcome to drive enumeration:
    /// an `Ingested`/`Skipped` day means "exists, keep going"; a `NotFound` (404) stops
    /// the day loop, and a 404 on day 1 means the round/venue does not exist (stops the
    /// round loop too). Per-meeting errors are counted as `failed` and do not abort the
    /// range. `interval` is awaited after each request that actually hit the network;
    /// pass `Duration::ZERO` (e.g. in tests) to disable the wait.
    pub async fn fetch_meeting_range(
        &self,
        range: &MeetingRange,
        force: bool,
        interval: Duration,
    ) -> Result<FetchRangeSummary> {
        let venues: Vec<Venue> = match range.venue {
            Some(v) => vec![v],
            None => Venue::all().to_vec(),
        };

        let mut summary = FetchRangeSummary::default();

        for venue in venues {
            let rounds: Vec<u32> = match range.round {
                Some(r) => vec![r],
                None => (1..=ROUND_CAP).collect(),
            };

            for round in rounds {
                let days: Vec<u32> = match range.day {
                    Some(d) => vec![d],
                    None => (1..=DAY_CAP).collect(),
                };

                let mut round_exists = true;
                for day in days {
                    let spec = MeetingSpec {
                        year: range.year,
                        round,
                        venue,
                        day,
                    };
                    let is_first_day = day == 1;

                    match self.fetch_meeting(&spec, force).await {
                        Ok(resp) => match resp.outcome {
                            FetchMeetingOutcome::Ingested {
                                races_saved,
                                horses_saved,
                            } => {
                                summary.ingested += 1;
                                summary.races_saved += races_saved;
                                summary.horses_saved += horses_saved;
                                self.wait(interval).await;
                            }
                            FetchMeetingOutcome::Skipped => {
                                summary.skipped += 1;
                                // history hit, no network request → no wait
                            }
                            FetchMeetingOutcome::NotFound => {
                                summary.not_found += 1;
                                self.wait(interval).await;
                                // No more days in this round; if day 1 is absent the
                                // round (and any later rounds for this venue) is absent.
                                if is_first_day && range.round.is_none() {
                                    round_exists = false;
                                }
                                break;
                            }
                        },
                        Err(e) => {
                            summary.failed += 1;
                            summary.failures.push((spec.source_key(), e.to_string()));
                            self.wait(interval).await;
                        }
                    }
                }

                // Round未指定で「この回が存在しない」と分かったら、以降の回も無い。
                if range.round.is_none() && !round_exists {
                    break;
                }
            }
        }

        Ok(summary)
    }

    async fn wait(&self, interval: Duration) {
        if !interval.is_zero() {
            tokio::time::sleep(interval).await;
        }
    }
}
