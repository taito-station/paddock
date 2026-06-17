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

/// Max time to establish the TCP/TLS connection before giving up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Deadline for the whole request — connect, response headers, and body read.
/// Without this, a stalled connection (no FIN, no data) blocks the calling
/// thread forever: a bulk `parse-pdf fetch` once hung ~8.7h on a single
/// mid-run network stall before this was added. See issue #152.
const GLOBAL_TIMEOUT: Duration = Duration::from_secs(60);
/// Total attempts (1 initial + 2 retries) for a transient failure.
const MAX_ATTEMPTS: u32 = 3;
/// Base backoff; attempt N waits `BASE * 2^(N-1)` (1s, 2s, …).
const RETRY_BASE_BACKOFF: Duration = Duration::from_secs(1);

/// Whether an error is worth retrying: transport-level hiccups and 5xx are
/// transient; 4xx (including the 403/404 "absent" answers) and malformed
/// requests are not.
fn is_transient(err: &ureq::Error) -> bool {
    match err {
        ureq::Error::Timeout(_)
        | ureq::Error::Io(_)
        | ureq::Error::ConnectionFailed
        | ureq::Error::HostNotFound
        | ureq::Error::Protocol(_) => true,
        ureq::Error::StatusCode(code) => *code >= 500,
        _ => false,
    }
}

pub struct UreqFetcher {
    /// Agent carrying the timeout config, reused across requests for connection
    /// pooling.
    agent: ureq::Agent,
    gate: RateGate,
}

impl Default for UreqFetcher {
    fn default() -> Self {
        Self::new(None)
    }
}

impl UreqFetcher {
    /// Build a fetcher whose outbound JRA requests are spaced at least
    /// `min_interval` apart, shared globally across concurrent fetch tasks.
    /// `None` (or [`UreqFetcher::default`]) imposes no rate cap — but timeouts
    /// and retries always apply.
    pub fn new(min_interval: Option<Duration>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_global(Some(GLOBAL_TIMEOUT))
            .build()
            .new_agent();
        Self {
            agent,
            gate: RateGate::new(min_interval),
        }
    }

    /// GET `url`, rate-gated, retrying transient failures with exponential
    /// backoff. The rate gate is honored before every attempt (each retry is a
    /// fresh network request and must stay within the JRA pacing cap).
    ///
    /// Only the response head is retried here; the body is read by the caller
    /// (`read_body`), so a stall mid-download is still bounded by the agent's
    /// `timeout_global` but surfaces as a one-shot `Io` error rather than being
    /// retried. Status errors are returned as-is: `fetch_if_exists` maps 403/404
    /// to "absent", while `fetch` surfaces them as errors.
    fn get_with_retry(&self, url: &str) -> std::result::Result<ureq::Body, ureq::Error> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            self.gate.wait();
            match self.agent.get(url).call() {
                Ok(resp) => return Ok(resp.into_body()),
                Err(err) if attempt < MAX_ATTEMPTS && is_transient(&err) => {
                    // Saturating so bumping MAX_ATTEMPTS can never overflow the
                    // shift or the Duration multiply into a panic.
                    let backoff =
                        RETRY_BASE_BACKOFF.saturating_mul(2u32.saturating_pow(attempt - 1));
                    tracing::warn!(
                        url,
                        attempt,
                        max_attempts = MAX_ATTEMPTS,
                        backoff_ms = backoff.as_millis() as u64,
                        error = %err,
                        "transient fetch error; retrying after backoff"
                    );
                    std::thread::sleep(backoff);
                }
                Err(err) => return Err(err),
            }
        }
    }
}

impl PdfFetcher for UreqFetcher {
    fn fetch(&self, url: &str) -> UcResult<Vec<u8>> {
        let body = self
            .get_with_retry(url)
            .map_err(|e| Error::Fetch(e.to_string()))?;
        read_body(body)
    }

    fn fetch_if_exists(&self, url: &str) -> UcResult<Option<Vec<u8>>> {
        match self.get_with_retry(url) {
            Ok(body) => Ok(Some(read_body(body)?)),
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
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread::{self, JoinHandle};

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

    #[test]
    fn is_transient_classifies_retryable_errors() {
        // 5xx and transport hiccups are retried.
        assert!(is_transient(&ureq::Error::StatusCode(500)));
        assert!(is_transient(&ureq::Error::StatusCode(503)));
        assert!(is_transient(&ureq::Error::ConnectionFailed));
        assert!(is_transient(&ureq::Error::HostNotFound));
        assert!(is_transient(&ureq::Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset"
        ))));
        // 4xx (including the "absent" 403/404) and client mistakes are not.
        assert!(!is_transient(&ureq::Error::StatusCode(404)));
        assert!(!is_transient(&ureq::Error::StatusCode(403)));
        assert!(!is_transient(&ureq::Error::StatusCode(400)));
        assert!(!is_transient(&ureq::Error::BadUri("nope".into())));
    }

    /// Minimal one-shot HTTP server: serves `responses` in order, one per
    /// accepted connection, closing each after replying. Returns the URL, a
    /// counter of accepted connections, and the join handle.
    fn serve(responses: Vec<&'static str>) -> (String, Arc<AtomicUsize>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&count);
        let handle = thread::spawn(move || {
            for resp in responses {
                let (mut stream, _) = listener.accept().unwrap();
                counter.fetch_add(1, Ordering::SeqCst);
                // Drain the request head enough to unblock the client's write.
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                stream.write_all(resp.as_bytes()).unwrap();
                stream.flush().unwrap();
                // stream dropped here → connection closed before next accept.
            }
        });
        (format!("http://{addr}/report.pdf"), count, handle)
    }

    const R_503: &str =
        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    const R_200_OK: &str = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
    const R_404: &str = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    #[test]
    fn retries_transient_5xx_then_succeeds() {
        // 503, 503, then 200 → fetch must retry twice and return the body.
        let (url, count, handle) = serve(vec![R_503, R_503, R_200_OK]);
        let fetcher = UreqFetcher::default();
        let body = fetcher.fetch(&url).expect("should succeed after retries");
        assert_eq!(body, b"ok");
        assert_eq!(
            count.load(Ordering::SeqCst),
            3,
            "expected 3 attempts (2 retries) before success"
        );
        handle.join().unwrap();
    }

    #[test]
    fn does_not_retry_404_and_reports_absent() {
        // 404 is "absent", not transient: fetch_if_exists returns None after a
        // single attempt (no retry).
        let (url, count, handle) = serve(vec![R_404]);
        let fetcher = UreqFetcher::default();
        let got = fetcher.fetch_if_exists(&url).expect("404 maps to Ok(None)");
        assert!(got.is_none());
        assert_eq!(count.load(Ordering::SeqCst), 1, "404 must not be retried");
        handle.join().unwrap();
    }
}
