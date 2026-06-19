use std::path::Path;
use std::time::Duration;

use paddock_domain::Venue;

use crate::dto::pdf::fetch::{
    DAY_CAP, FetchMeetingOutcome, FetchMeetingResponse, FetchRangeSummary, MeetingRange,
    MeetingSpec, ROUND_CAP,
};
use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::{FetchProbe, PdfFetcher};
use crate::pdf_parser::PdfParser;
use crate::repository::{
    FetchDownload, FetchFailure, FetchRecord, FetchRepository, FetchStatus, RaceRepository,
};

impl<R: RaceRepository + FetchRepository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    /// Fetch a single JRA meeting-day result PDF.
    ///
    /// Two stages share this method, selected by `inbox` (#147):
    /// - `inbox = None` (one-shot / Stage2-equivalent): parse in memory and store
    ///   the races, keeping only a fetch-history row (`ingested`). The PDF is
    ///   never written to disk. Skipped when already **ingested**.
    /// - `inbox = Some(dir)` (Stage1, `--download-only`): write the PDF to
    ///   `dir/{pdf_filename}` and record it `downloaded` **without parsing**, so a
    ///   later `ingest` run does the parse. Skipped when already downloaded or
    ///   ingested.
    ///
    /// A non-existent PDF (HTTP 403/404) reports [`FetchMeetingOutcome::NotFound`]
    /// and is *not* recorded, so it can be retried once JRA publishes it.
    pub async fn fetch_meeting(
        &self,
        spec: &MeetingSpec,
        force: bool,
        inbox: Option<&Path>,
    ) -> Result<FetchMeetingResponse> {
        let source_key = spec.source_key();
        let url = spec.pdf_url();

        // dedup: Stage1 (download-only) skips a meeting already downloaded *or*
        // ingested; the one-shot path only skips once it has been ingested. A
        // `failed` row (#170) is NOT a skip — it is a re-fetch candidate, so the
        // Stage1 check matches only the two success states, not `Some(_)`.
        if !force {
            let already = match inbox {
                Some(_) => matches!(
                    self.repository.fetch_status(&source_key).await?,
                    Some(FetchStatus::Downloaded | FetchStatus::Ingested)
                ),
                None => self.repository.fetch_history_contains(&source_key).await?,
            };
            if already {
                return Ok(FetchMeetingResponse {
                    source_key,
                    url,
                    outcome: FetchMeetingOutcome::Skipped,
                });
            }
        }

        let bytes = match self.pdf_fetcher.fetch_if_exists(&url)? {
            FetchProbe::Found(bytes) => bytes,
            // Absent (403/404). Surface the status but do NOT record here: a single
            // `fetch_meeting` has no adjacency knowledge, so the parallel grid path
            // never persists junk. Only the sequential range loop, which knows a
            // day follows confirmed successes, records a boundary absence as failed.
            FetchProbe::Absent(http_status) => {
                return Ok(FetchMeetingResponse {
                    source_key,
                    url,
                    outcome: FetchMeetingOutcome::NotFound { http_status },
                });
            }
        };

        // Stage1: write the raw PDF to inbox and record `downloaded`; no parse.
        // Order matters: write the file *before* recording, so a crash between the
        // two leaves a row-less inbox file that the next run re-downloads (its
        // `fetch_status` is None) rather than a `downloaded` row pointing at a
        // missing file. fetch_history is the source of truth; a missing inbox file
        // for a `downloaded` row is recovered by re-fetching (`--force` or #170).
        if let Some(inbox_dir) = inbox {
            let path = inbox_dir.join(spec.pdf_filename());
            std::fs::create_dir_all(inbox_dir).map_err(|e| {
                crate::Error::Internal(format!("create inbox dir {}: {e}", inbox_dir.display()))
            })?;
            std::fs::write(&path, &bytes)
                .map_err(|e| crate::Error::Internal(format!("write {}: {e}", path.display())))?;
            self.repository
                .record_download(&FetchDownload {
                    source_key: source_key.clone(),
                    url: url.clone(),
                    downloaded_at: chrono::Utc::now(),
                })
                .await?;
            return Ok(FetchMeetingResponse {
                source_key,
                url,
                outcome: FetchMeetingOutcome::Downloaded { path },
            });
        }

        let races = self.pdf_parser.parse(&bytes)?;

        // A 0-race parse is treated as a failure, not a success: the PDF exists
        // (it was fetched), so recording it in fetch history would mark the
        // meeting "done" and make every later fetch skip it without hitting the
        // network — silently self-blocking re-acquisition. Leave no history row
        // so it stays a re-fetch candidate. See issue #149.
        if races.is_empty() {
            // Log which meeting was parser-gapped: the range summary only carries
            // an empty count, so this is the trail that names the source_key.
            tracing::info!(%source_key, %url, "fetched but parsed 0 races; not recorded (re-fetchable)");
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
        inbox: Option<&Path>,
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
                // Count of confirmed-existing days seen so far in this round
                // (Ingested/Downloaded/Skipped/Empty). A 403/404 that follows ≥1 of
                // these is the "連続成功直後の単発 403/404" boundary — a day that
                // plausibly exists (not-yet-published or a transient JRA block), so it
                // is persisted as a retryable `failed` row. A 403/404 with none before
                // it (day-1 absent) means the round itself does not exist and is left
                // unrecorded — keeping grid junk out of fetch_history (#170).
                let mut existing_in_round = 0u32;
                for day in days {
                    let spec = MeetingSpec {
                        year: range.year,
                        round,
                        venue,
                        day,
                    };
                    let is_first_day = day == 1;

                    match self.fetch_meeting(&spec, force, inbox).await {
                        Ok(resp) => match resp.outcome {
                            FetchMeetingOutcome::Ingested {
                                races_saved,
                                horses_saved,
                            } => {
                                summary.ingested += 1;
                                summary.races_saved += races_saved;
                                summary.horses_saved += horses_saved;
                                existing_in_round += 1;
                                self.wait(interval).await;
                            }
                            FetchMeetingOutcome::Downloaded { .. } => {
                                // Stage1: the PDF exists and was written to inbox.
                                // Behaves like Ingested for boundary discovery
                                // ("exists, keep going") and waits after the GET.
                                summary.downloaded += 1;
                                existing_in_round += 1;
                                self.wait(interval).await;
                            }
                            FetchMeetingOutcome::Skipped => {
                                summary.skipped += 1;
                                existing_in_round += 1;
                                // history hit, no network request → no wait
                            }
                            FetchMeetingOutcome::Empty => {
                                // PDF exists but parsed to 0 races: count it and
                                // keep going (it is not a round/day boundary).
                                // The PDF was downloaded (network round-trip), so
                                // wait afterwards like the other fetched outcomes.
                                summary.empty += 1;
                                existing_in_round += 1;
                                self.wait(interval).await;
                            }
                            FetchMeetingOutcome::NotFound { http_status } => {
                                summary.not_found += 1;
                                // A boundary absence right after ≥1 existing day in this
                                // round is a plausibly-real day (not-yet-published or a
                                // transient block): persist it as a retryable `failed`
                                // row. A day-1 absence (no prior existing day) is the
                                // round-nonexistence boundary — leave it unrecorded.
                                // (A pinned single-day fetch, `range.day = Some`, also has
                                // existing_in_round == 0, so it is never recorded here.)
                                //
                                // Recording is best-effort: a DB hiccup must not abort a
                                // long bulk fetch (the whole point of #170 is resilience).
                                // Mirror the sibling write paths, whose errors surface from
                                // fetch_meeting into the loop's `Err` arm and continue; the
                                // boundary is re-recorded on the next run, so it stays idempotent.
                                if existing_in_round > 0 {
                                    match self
                                        .repository
                                        .record_failure(&FetchFailure {
                                            source_key: spec.source_key(),
                                            url: spec.pdf_url(),
                                            http_status,
                                            attempted_at: chrono::Utc::now(),
                                        })
                                        .await
                                    {
                                        Ok(()) => {
                                            summary.recorded_failed += 1;
                                            tracing::info!(
                                                source_key = %spec.source_key(),
                                                http_status,
                                                "boundary 403/404 recorded as failed (re-fetchable)"
                                            );
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                source_key = %spec.source_key(),
                                                http_status,
                                                error = %e,
                                                "failed to record boundary 403/404; continuing (re-recorded next run)"
                                            );
                                        }
                                    }
                                }
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
