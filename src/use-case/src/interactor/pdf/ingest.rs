use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::dto::pdf::fetch::MeetingSpec;
use crate::dto::pdf::ingest::IngestPdfResponse;
use crate::error::Result;
use crate::interactor::Interactor;
use crate::pdf_fetcher::PdfFetcher;
use crate::pdf_parser::PdfParser;
use crate::repository::{FetchRecord, FetchRepository, RaceRepository};

impl<R: RaceRepository + FetchRepository, P: PdfParser, F: PdfFetcher> Interactor<R, P, F> {
    pub async fn ingest_pdf(&self, source: &str) -> Result<IngestPdfResponse> {
        let bytes = if source.starts_with("http://") || source.starts_with("https://") {
            self.pdf_fetcher.fetch(source)?
        } else {
            std::fs::read(source).map_err(|e| {
                crate::Error::InvalidArgument(format!("failed to read {source}: {e}"))
            })?
        };
        let races = self.pdf_parser.parse(&bytes)?;
        let mut races_saved = 0;
        let mut horses_saved = 0;
        for race in &races {
            self.repository.save_race(race).await?;
            races_saved += 1;
            horses_saved += race.results.len();
        }

        // Stage2 (#147): an inbox meeting PDF that ingested at least one race is
        // recorded `ingested` and removed, so fetch_history is the single source of
        // truth (the inbox→done move is retired for results). A 0-race parse is left
        // in place and unrecorded — a parser gap that stays re-ingestable (#149).
        if races_saved > 0
            && let Some((spec, path)) = inbox_meeting(source)
        {
            self.repository
                .record_fetch(&FetchRecord {
                    source_key: spec.source_key(),
                    url: spec.pdf_url(),
                    races_saved: races_saved as u32,
                    horses_saved: horses_saved as u32,
                    fetched_at: Utc::now(),
                })
                .await?;
            // Recording precedes deletion: if removal fails the row is already
            // `ingested`, so the leftover PDF is harmless — the next ingest run
            // re-parses it, the ON CONFLICT upsert is idempotent, and deletion is
            // retried. Hence a warn (not an error) is enough here.
            if let Err(e) = std::fs::remove_file(&path) {
                tracing::warn!(
                    source,
                    error = %e,
                    "ingested but failed to remove inbox PDF; the next ingest run will retry removal"
                );
            }
        }

        Ok(IngestPdfResponse {
            races_saved,
            horses_saved,
        })
    }
}

/// If `source` is a local file directly under an `inbox/` directory whose name
/// parses to a [`MeetingSpec`], return the spec and the file's canonical path.
/// Used by Stage2 to record the meeting `ingested` and delete the inbox PDF.
/// Returns `None` for http sources, files not under `inbox/`, and non-meeting
/// file names — those are ingested but left in place (no record, no delete).
fn inbox_meeting(source: &str) -> Option<(MeetingSpec, PathBuf)> {
    if source.starts_with("http://") || source.starts_with("https://") {
        return None;
    }
    let canonical = Path::new(source).canonicalize().ok()?;
    let parent = canonical.parent()?;
    if parent.file_name().and_then(|n| n.to_str()) != Some("inbox") {
        return None;
    }
    let name = canonical.file_name().and_then(|n| n.to_str())?;
    let spec = MeetingSpec::from_pdf_filename(name)?;
    Some((spec, canonical))
}
