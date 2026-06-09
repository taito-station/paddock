mod cli;
mod setup;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use paddock_domain::Venue;
use paddock_use_case::dto::pdf::fetch::{
    FetchMeetingOutcome, FetchMeetingResponse, FetchRangeSummary, MeetingRange, MeetingSpec,
};
use paddock_use_case::dto::pdf::ingest::IngestPdfResponse;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::Instrument;

use cli::{Cli, Command, FetchArgs, IngestArgs};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let app = Arc::new(setup::build_app().await?);

    match cli.command.unwrap_or(Command::Ingest(cli.ingest)) {
        Command::Ingest(args) => run_ingest(app, args).await,
        Command::Fetch(args) => run_fetch(app, args).await,
    }
}

async fn run_ingest(app: Arc<setup::App>, args: IngestArgs) -> anyhow::Result<()> {
    let parallel = resolve_parallel(args.parallel);
    let total = args.sources.len();
    // Pin OCR to one thread per process when several PDFs OCR concurrently, mirroring
    // the fetch path. Must run before spawning the worker tasks below (see SAFETY).
    if should_pin_ocr(parallel, total) {
        pin_ocr_to_single_thread();
    }
    let semaphore = Arc::new(Semaphore::new(parallel));
    let mut joinset: JoinSet<(String, paddock_use_case::Result<IngestPdfResponse>)> =
        JoinSet::new();

    for source in args.sources {
        let app = Arc::clone(&app);
        let permit = Arc::clone(&semaphore)
            .acquire_owned()
            .await
            .context("acquire semaphore permit")?;
        let span = tracing::info_span!("ingest", source = %source);
        joinset.spawn(
            async move {
                let _permit = permit;
                let result = app.ingest_pdf(&source).await;
                (source, result)
            }
            .instrument(span),
        );
    }

    let mut succeeded = 0usize;
    let mut failed = 0usize;
    while let Some(joined) = joinset.join_next().await {
        let (source, result) = joined?;
        match result {
            Ok(response) => {
                println!(
                    "ingested: {} race(s), {} horse result(s) from {}",
                    response.races_saved, response.horses_saved, source
                );
                match move_to_done_if_inbox(&source) {
                    Ok(Some(dest)) => println!("moved: {} -> {}", source, dest.display()),
                    Ok(None) => {}
                    Err(e) => eprintln!("warn: failed to move {source}: {e}"),
                }
                succeeded += 1;
            }
            Err(e) => {
                eprintln!("error: failed to ingest {source}: {e}");
                failed += 1;
            }
        }
    }

    if total > 1 {
        println!("---");
        println!("done: {succeeded}/{total} succeeded (parallel={parallel})");
    }

    if failed > 0 {
        anyhow::bail!("{failed} of {total} files failed");
    }
    Ok(())
}

async fn run_fetch(app: Arc<setup::App>, args: FetchArgs) -> anyhow::Result<()> {
    // Progressive omission: a narrower field requires every broader one.
    if args.day.is_some() && args.round.is_none() {
        anyhow::bail!("--day requires --round (and --venue)");
    }
    if args.round.is_some() && args.venue.is_none() {
        anyhow::bail!("--round requires --venue");
    }

    let venue = match &args.venue {
        Some(v) => {
            Some(Venue::try_from(v.as_str()).with_context(|| format!("invalid venue: {v}"))?)
        }
        None => None,
    };

    // All four fields present → single meeting (preserves the original behavior).
    if let (Some(venue), Some(round), Some(day)) = (venue, args.round, args.day) {
        return run_fetch_single(app, args.year, venue, round, day, args.force).await;
    }

    // Otherwise → range fetch with a summary.
    let range = MeetingRange {
        year: args.year,
        venue,
        round: args.round,
        day: args.day,
    };

    let parallel = resolve_parallel(args.parallel);
    // Warn when the user appears to have set a non-default --interval that the
    // parallel path will ignore. We compare against the clap default (1.0) as a
    // proxy for "explicitly set" — passing exactly --interval 1 is treated as default.
    if parallel > 1 && args.interval != 1.0 {
        eprintln!(
            "note: --interval is ignored when fetching in parallel; use -j 1 for sequential pacing"
        );
    }
    let summary = if parallel > 1 {
        run_fetch_range_parallel(app, range, args.force, parallel).await?
    } else {
        // Sequential: keep the smart 404-boundary discovery and the polite interval.
        let interval = Duration::from_secs_f64(args.interval.max(0.0));
        let span = tracing::info_span!("fetch_range", year = args.year);
        app.fetch_meeting_range(&range, args.force, interval)
            .instrument(span)
            .await?
    };

    println!(
        "done: {} ingested, {} skipped, {} not-found, {} failed ({} race(s), {} horse result(s))",
        summary.ingested,
        summary.skipped,
        summary.not_found,
        summary.failed,
        summary.races_saved,
        summary.horses_saved,
    );
    if !summary.failures.is_empty() {
        eprintln!("failures:");
        for (key, err) in &summary.failures {
            eprintln!("  {key}: {err}");
        }
        anyhow::bail!("{} meeting(s) failed", summary.failed);
    }
    Ok(())
}

async fn run_fetch_single(
    app: Arc<setup::App>,
    year: i32,
    venue: Venue,
    round: u32,
    day: u32,
    force: bool,
) -> anyhow::Result<()> {
    let spec = MeetingSpec {
        year,
        round,
        venue,
        day,
    };

    let span = tracing::info_span!("fetch", source_key = %spec.source_key());
    let response = app.fetch_meeting(&spec, force).instrument(span).await?;

    match response.outcome {
        FetchMeetingOutcome::Ingested {
            races_saved,
            horses_saved,
        } => {
            println!(
                "ingested: {races_saved} race(s), {horses_saved} horse result(s) from {}",
                response.url
            );
        }
        FetchMeetingOutcome::Skipped => {
            println!(
                "skipped: {} already ingested (use --force to re-fetch)",
                response.source_key
            );
        }
        FetchMeetingOutcome::NotFound => {
            anyhow::bail!("not found: {} (no PDF; HTTP 403/404)", response.url);
        }
    }
    Ok(())
}

/// Range fetch, parallelized. Unlike the sequential `fetch_meeting_range`, this
/// enumerates the whole candidate grid (`range.candidate_specs()`) and fetches up
/// to `parallel` meetings at once, reusing `fetch_meeting` so each ingested meeting
/// is still recorded in `fetch_history`. Discovery 403/404s are cheap and counted
/// as not-found; already-ingested meetings short-circuit as skipped (no network).
///
/// Unlike the sequential path it emits no truncation warning when a meeting reaches
/// `ROUND_CAP`/`DAY_CAP`; the caps sit above real JRA maxima, so the grid is expected
/// to cover every published day.
///
/// Concurrency mirrors `run_ingest` (owned-permit `Semaphore` + `JoinSet`).
async fn run_fetch_range_parallel(
    app: Arc<setup::App>,
    range: MeetingRange,
    force: bool,
    parallel: usize,
) -> anyhow::Result<FetchRangeSummary> {
    // This path always runs with `parallel > 1` over a multi-candidate grid, so pin
    // OCR to one thread per process unconditionally.
    pin_ocr_to_single_thread();

    let specs = range.candidate_specs();
    let semaphore = Arc::new(Semaphore::new(parallel));
    let mut joinset: JoinSet<(String, paddock_use_case::Result<FetchMeetingResponse>)> =
        JoinSet::new();

    for spec in specs {
        let app = Arc::clone(&app);
        let permit = Arc::clone(&semaphore)
            .acquire_owned()
            .await
            .context("acquire semaphore permit")?;
        let span = tracing::info_span!("fetch", source_key = %spec.source_key());
        joinset.spawn(
            async move {
                let _permit = permit;
                let key = spec.source_key();
                let result = app.fetch_meeting(&spec, force).await;
                (key, result)
            }
            .instrument(span),
        );
    }

    let mut summary = FetchRangeSummary::default();
    while let Some(joined) = joinset.join_next().await {
        let (key, result) = joined?;
        accumulate_outcome(&mut summary, key, result);
    }

    Ok(summary)
}

/// Fold one meeting fetch result into the running range summary. Kept pure (no
/// IO / no concurrency) so the parallel aggregation can be unit-tested directly.
fn accumulate_outcome(
    summary: &mut FetchRangeSummary,
    key: String,
    result: paddock_use_case::Result<FetchMeetingResponse>,
) {
    match result {
        Ok(resp) => match resp.outcome {
            FetchMeetingOutcome::Ingested {
                races_saved,
                horses_saved,
            } => {
                summary.ingested += 1;
                summary.races_saved += races_saved;
                summary.horses_saved += horses_saved;
            }
            FetchMeetingOutcome::Skipped => summary.skipped += 1,
            FetchMeetingOutcome::NotFound => summary.not_found += 1,
        },
        Err(e) => {
            summary.failed += 1;
            summary.failures.push((key, e.to_string()));
        }
    }
}

fn resolve_parallel(requested: Option<usize>) -> usize {
    requested
        .or_else(|| std::thread::available_parallelism().ok().map(|n| n.get()))
        .unwrap_or(4)
        .max(1)
}

/// Whether to pin OCR to one thread per process for this batch. We pin only when
/// several items actually OCR concurrently (`parallel > 1 && count > 1`): with N
/// tesseracts each grabbing all cores the CPU is oversubscribed (parallel × cores
/// threads). A lone item (`parse-pdf one.pdf`) or a sequential run (`-j 1`) has no
/// contention, so it keeps all cores.
fn should_pin_ocr(parallel: usize, count: usize) -> bool {
    parallel > 1 && count > 1
}

/// Pin OCR (tesseract/OpenMP) to one thread per process via env vars. tesseract
/// inherits these (its Command is not env_clear'd), so one thread per process fills
/// the cores cleanly (N processes × 1 thread). Guard the call with [`should_pin_ocr`]
/// so a lone PDF/meeting keeps all cores.
///
/// SAFETY: although the tokio runtime is multi-threaded, nothing else in this process
/// reads or writes the environment, and callers MUST invoke this before spawning the
/// OCR worker tasks, so no observer races these writes.
fn pin_ocr_to_single_thread() {
    unsafe {
        std::env::set_var("OMP_THREAD_LIMIT", "1");
        std::env::set_var("OMP_NUM_THREADS", "1");
    }
}

/// Move a successfully ingested file from `<root>/pdfs/<kind>/inbox/<file>` to
/// `<root>/pdfs/<kind>/done/<file>`. Detection is based on src's parent directory chain,
/// so it works regardless of CWD and across PDF kinds (results, entries, ...).
fn move_to_done_if_inbox(source: &str) -> anyhow::Result<Option<PathBuf>> {
    if source.starts_with("http://") || source.starts_with("https://") {
        return Ok(None);
    }
    let src = Path::new(source);
    let Ok(canonical_src) = src.canonicalize() else {
        return Ok(None);
    };
    let Some(parent) = canonical_src.parent() else {
        return Ok(None);
    };
    if parent.file_name().and_then(|n| n.to_str()) != Some("inbox") {
        return Ok(None);
    }
    let Some(kind_dir) = parent.parent() else {
        return Ok(None);
    };
    let Some(pdfs_dir) = kind_dir.parent() else {
        return Ok(None);
    };
    if pdfs_dir.file_name().and_then(|n| n.to_str()) != Some("pdfs") {
        return Ok(None);
    }
    let file_name = canonical_src
        .file_name()
        .context("source has no file name")?;
    let done = kind_dir.join("done");
    std::fs::create_dir_all(&done).with_context(|| format!("create {}", done.display()))?;
    let dest = done.join(file_name);
    std::fs::rename(&canonical_src, &dest)
        .with_context(|| format!("rename {} -> {}", canonical_src.display(), dest.display()))?;
    Ok(Some(dest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_pin_ocr_only_when_parallel_and_multiple() {
        assert!(should_pin_ocr(4, 8)); // parallel batch → pin
        assert!(!should_pin_ocr(1, 8)); // `-j 1` sequential → keep all cores
        assert!(!should_pin_ocr(4, 1)); // single PDF → keep all cores
        assert!(!should_pin_ocr(1, 1));
    }

    fn resp(key: &str, outcome: FetchMeetingOutcome) -> FetchMeetingResponse {
        FetchMeetingResponse {
            source_key: key.to_string(),
            url: format!("https://example/{key}.pdf"),
            outcome,
        }
    }

    #[test]
    fn accumulate_outcome_folds_each_variant() {
        let mut s = FetchRangeSummary::default();

        accumulate_outcome(
            &mut s,
            "k1".into(),
            Ok(resp(
                "k1",
                FetchMeetingOutcome::Ingested {
                    races_saved: 12,
                    horses_saved: 200,
                },
            )),
        );
        accumulate_outcome(
            &mut s,
            "k2".into(),
            Ok(resp("k2", FetchMeetingOutcome::Skipped)),
        );
        accumulate_outcome(
            &mut s,
            "k3".into(),
            Ok(resp("k3", FetchMeetingOutcome::NotFound)),
        );
        accumulate_outcome(
            &mut s,
            "k4".into(),
            Err(paddock_use_case::Error::Internal("boom".into())),
        );

        assert_eq!(s.ingested, 1);
        assert_eq!(s.races_saved, 12);
        assert_eq!(s.horses_saved, 200);
        assert_eq!(s.skipped, 1);
        assert_eq!(s.not_found, 1);
        assert_eq!(s.failed, 1);
        // failed meetings retain their key + error so run_fetch can list them.
        assert_eq!(s.failures.len(), 1);
        assert_eq!(s.failures[0].0, "k4");
        assert!(s.failures[0].1.contains("boom"));
    }
}
