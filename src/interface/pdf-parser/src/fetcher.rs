use std::io::Read;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use paddock_use_case::Result as UcResult;
use paddock_use_case::pdf_fetcher::PdfFetcher;

use crate::error::Error;

/// Global minimum spacing between outbound JRA requests, shared across every
/// concurrent fetch task (the single [`UreqFetcher`] is shared via `Arc<App>`).
/// A `None` interval disables throttling (the default). History skips never
/// reach the fetcher, so only real network GETs are spaced — re-runs that hit
/// `fetch_history` stay fast.
#[derive(Default)]
struct RateGate {
    /// Minimum time between request *starts*. `None` = no cap.
    min_interval: Option<Duration>,
    /// Start time of the most recent request, shared across tasks.
    last: Mutex<Option<Instant>>,
}

impl RateGate {
    fn new(min_interval: Option<Duration>) -> Self {
        Self {
            min_interval,
            last: Mutex::new(None),
        }
    }

    /// Block until at least `min_interval` has elapsed since the previous
    /// request start, then record now as the latest start. The lock is held
    /// across the sleep so concurrent callers serialize their starts and the
    /// global rate stays under the cap.
    ///
    /// The wait is a blocking `thread::sleep`, matching the fetcher's existing
    /// blocking ureq/OCR pattern (the parallel range fetch bounds concurrency by
    /// CPU cores, so worker threads already block during fetch). This assumes
    /// in-flight fetches stay around the CPU-core count (the current `Semaphore`
    /// bound); pushing concurrency far beyond that would park many runtime threads
    /// here and should instead move to `spawn_blocking` / async sleep.
    fn wait(&self) {
        let Some(min) = self.min_interval else {
            return;
        };
        let mut last = self.last.lock().expect("rate gate mutex poisoned");
        if let Some(prev) = *last {
            let elapsed = prev.elapsed();
            if elapsed < min {
                std::thread::sleep(min - elapsed);
            }
        }
        *last = Some(Instant::now());
    }
}

#[derive(Default)]
pub struct UreqFetcher {
    gate: RateGate,
}

impl UreqFetcher {
    /// Build a fetcher whose outbound JRA requests are spaced at least
    /// `min_interval` apart, shared globally across concurrent fetch tasks.
    /// `None` (or [`UreqFetcher::default`]) imposes no cap — the original
    /// behavior.
    pub fn new(min_interval: Option<Duration>) -> Self {
        Self {
            gate: RateGate::new(min_interval),
        }
    }
}

impl PdfFetcher for UreqFetcher {
    fn fetch(&self, url: &str) -> UcResult<Vec<u8>> {
        self.gate.wait();
        let resp = ureq::get(url)
            .call()
            .map_err(|e| Error::Fetch(e.to_string()))?;
        read_body(resp.into_body())
    }

    fn fetch_if_exists(&self, url: &str) -> UcResult<Option<Vec<u8>>> {
        self.gate.wait();
        match ureq::get(url).call() {
            Ok(resp) => Ok(Some(read_body(resp.into_body())?)),
            // A meeting PDF that does not exist is reported as absent so range
            // enumeration can stop / skip instead of erroring. JRA's seiseki
            // directory answers a missing report with 403 (not just 404): a
            // not-yet-published day returns 404, while a never-existing
            // (round/day past the meeting, or a non-running venue) returns 403.
            // Both mean "no PDF here", so treat them alike.
            Err(ureq::Error::StatusCode(403 | 404)) => Ok(None),
            Err(e) => Err(Error::Fetch(e.to_string()).into()),
        }
    }
}

fn read_body(body: ureq::Body) -> UcResult<Vec<u8>> {
    let mut buf = Vec::new();
    body.into_reader()
        .read_to_end(&mut buf)
        .map_err(Error::Io)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_gate_spaces_consecutive_waits() {
        // 40ms spacing: first wait is free (no prior), the next two each block
        // ~40ms, so three waits take at least ~80ms.
        let gate = RateGate::new(Some(Duration::from_millis(40)));
        let start = Instant::now();
        gate.wait();
        gate.wait();
        gate.wait();
        assert!(
            start.elapsed() >= Duration::from_millis(80),
            "expected >=80ms across three 40ms-spaced waits, got {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn rate_gate_none_never_blocks() {
        let gate = RateGate::new(None);
        let start = Instant::now();
        for _ in 0..5 {
            gate.wait();
        }
        assert!(
            start.elapsed() < Duration::from_millis(20),
            "an unlimited gate must not sleep, took {:?}",
            start.elapsed()
        );
    }
}
