use crate::dto::pdf::fetch::{FetchMeetingOutcome, FetchMeetingResponse, MeetingSpec};
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
}
