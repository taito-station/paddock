use std::time::Duration;

use paddock_domain::Venue;

use crate::dto::pdf::fetch::{
    DAY_CAP, FetchMeetingOutcome, FetchMeetingResponse, FetchRangeSummary, MeetingRange,
    MeetingSpec, ROUND_CAP,
};
use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{FetchRecord, Repository};

impl<R: Repository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// Fetch a single JRA meeting-day result PDF, parse it, and store the
    /// races. The PDF itself is never written to disk — only a fetch-history
    /// row is kept so the same meeting is not re-ingested on a later run.
    ///
    /// When `force` is false and the meeting is already in fetch history, the
    /// fetch is skipped entirely. A non-existent PDF (HTTP 403/404) reports
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

        // A 0-race parse is treated as a failure, not a success: the PDF exists
        // (it was fetched), so recording it in fetch history would mark the
        // meeting "done" and make every later fetch skip it without hitting the
        // network — silently self-blocking re-acquisition. Leave no history row
        // so it stays a re-fetch candidate. See issue #149.
        if races.is_empty() {
            return Ok(FetchMeetingResponse {
                source_key,
                url,
                outcome: FetchMeetingOutcome::Empty,
            });
        }

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
    /// an `Ingested`/`Skipped` day means "exists, keep going"; a `NotFound` (403/404)
    /// stops the day loop, and a not-found day 1 means the round/venue does not exist
    /// (stops the round loop too). Per-meeting errors are counted as `failed` and do not abort the
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

            let mut hit_round_boundary = false;
            for round in rounds {
                let days: Vec<u32> = match range.day {
                    Some(d) => vec![d],
                    None => (1..=DAY_CAP).collect(),
                };

                let mut round_exists = true;
                let mut hit_day_boundary = false;
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
                            FetchMeetingOutcome::Empty => {
                                // PDF exists but parsed to 0 races: count it and
                                // keep going (it is not a round/day boundary).
                                summary.empty += 1;
                                self.wait(interval).await;
                            }
                            FetchMeetingOutcome::NotFound => {
                                summary.not_found += 1;
                                self.wait(interval).await;
                                // No more days in this round; if day 1 is absent the
                                // round (and any later rounds for this venue) is absent.
                                if is_first_day && range.round.is_none() {
                                    round_exists = false;
                                }
                                hit_day_boundary = true;
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

                // The day loop is meant to stop at a not-found (403/404) boundary. If it
                // instead ran out at DAY_CAP (no boundary seen) while the range was
                // open-ended, the meeting may have more days than the guard allows —
                // surface it rather than silently truncating.
                if range.day.is_none() && !hit_day_boundary {
                    tracing::warn!(
                        year = range.year,
                        %venue,
                        round,
                        day_cap = DAY_CAP,
                        "day cap reached without a not-found boundary; results may be truncated"
                    );
                }

                // Round未指定で「この回が存在しない」と分かったら、以降の回も無い。
                if range.round.is_none() && !round_exists {
                    hit_round_boundary = true;
                    break;
                }
            }

            // Round enumeration normally stops at an absent round (the boundary). If it
            // instead ran out at ROUND_CAP, the venue may hold more rounds than the guard
            // allows — surface it rather than silently truncating.
            if range.round.is_none() && !hit_round_boundary {
                tracing::warn!(
                    year = range.year,
                    %venue,
                    round_cap = ROUND_CAP,
                    "round cap reached without an absent-round boundary; more rounds may exist"
                );
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
