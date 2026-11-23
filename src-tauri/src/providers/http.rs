//! Shared HTTP helpers for providers: polite rate limiting for scrapers and
//! HTTP 429 handling (exponential backoff with jitter, `Retry-After` support).

use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::header::RETRY_AFTER;
use reqwest::{RequestBuilder, Response};
use tokio::sync::Mutex;

use crate::error::AppError;

/// Total attempts for one logical request (1 initial + 2 retries).
pub const MAX_ATTEMPTS: u32 = 3;
const BASE_BACKOFF_MS: u64 = 800;
const MAX_WAIT: Duration = Duration::from_secs(20);
/// At most 2 requests per second per scraped provider.
const MIN_INTERVAL: Duration = Duration::from_millis(500);

pub fn user_agent() -> String {
    format!(
        "SFX-de-Babel/{} (desktop SFX search tool; polite client, max 2 req/s)",
        env!("CARGO_PKG_VERSION")
    )
}

/// Serializes requests to one host so they are at least [`MIN_INTERVAL`] apart.
pub struct RateGate {
    last: Mutex<Option<Instant>>,
}

impl RateGate {
    pub const fn new() -> Self {
        Self {
            last: Mutex::const_new(None),
        }
    }

    pub async fn wait(&self) {
        let mut last = self.last.lock().await;
        if let Some(prev) = *last {
            let elapsed = prev.elapsed();
            if elapsed < MIN_INTERVAL {
                tokio::time::sleep(MIN_INTERVAL - elapsed).await;
            }
        }
        *last = Some(Instant::now());
    }
}

pub fn mixkit_gate() -> &'static RateGate {
    static GATE: OnceLock<RateGate> = OnceLock::new();
    GATE.get_or_init(RateGate::new)
}

pub fn freesound_web_gate() -> &'static RateGate {
    static GATE: OnceLock<RateGate> = OnceLock::new();
    GATE.get_or_init(RateGate::new)
}

/// Parses a `Retry-After` header expressed in seconds. (HTTP-date form is ignored
/// and falls back to exponential backoff.)
pub fn parse_retry_after(raw: &str) -> Option<Duration> {
    raw.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// Delay before retry number `attempt` (1-based: the wait after the first failure is `attempt = 1`).
///
/// `jitter_unit` is in `[0, 1)`; it adds up to +50% to the exponential delay.
/// A server-provided `Retry-After` always wins (capped at [`MAX_WAIT`]).
pub fn backoff_delay(attempt: u32, retry_after: Option<Duration>, jitter_unit: f64) -> Duration {
    if let Some(server) = retry_after {
        return server.min(MAX_WAIT);
    }
    let exp = BASE_BACKOFF_MS.saturating_mul(1u64 << attempt.saturating_sub(1).min(6));
    let jitter = (exp as f64 * 0.5 * jitter_unit.clamp(0.0, 1.0)) as u64;
    Duration::from_millis(exp + jitter).min(MAX_WAIT)
}

fn jitter_unit() -> f64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (nanos % 1000) as f64 / 1000.0
}

/// Sends a request built by `make`, retrying on HTTP 429 up to [`MAX_ATTEMPTS`] times.
/// Returns [`AppError::RateLimited`] when every attempt was rate limited.
/// Other statuses are returned to the caller untouched.
pub async fn send_with_retry<F>(
    provider: &'static str,
    gate: Option<&RateGate>,
    make: F,
) -> Result<Response, AppError>
where
    F: Fn() -> RequestBuilder,
{
    let mut last_retry_after: Option<u64> = None;
    for attempt in 1..=MAX_ATTEMPTS {
        if let Some(g) = gate {
            g.wait().await;
        }
        let resp = make().send().await?;
        if resp.status().as_u16() != 429 {
            return Ok(resp);
        }

        let retry_after = resp
            .headers()
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_retry_after);
        last_retry_after = retry_after.map(|d| d.as_secs());

        if attempt < MAX_ATTEMPTS {
            tokio::time::sleep(backoff_delay(attempt, retry_after, jitter_unit())).await;
        }
    }
    Err(AppError::RateLimited {
        provider: provider.to_string(),
        retry_after_secs: last_retry_after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_exponentially_with_bounded_jitter() {
        let d1 = backoff_delay(1, None, 0.0);
        let d2 = backoff_delay(2, None, 0.0);
        let d3 = backoff_delay(3, None, 0.0);
        assert_eq!(d1, Duration::from_millis(800));
        assert_eq!(d2, Duration::from_millis(1600));
        assert_eq!(d3, Duration::from_millis(3200));

        let with_jitter = backoff_delay(2, None, 0.999);
        assert!(with_jitter > d2 && with_jitter <= d2 + d2 / 2);
    }

    #[test]
    fn retry_after_overrides_backoff_and_is_capped() {
        assert_eq!(
            backoff_delay(1, Some(Duration::from_secs(3)), 0.9),
            Duration::from_secs(3)
        );
        assert_eq!(backoff_delay(1, Some(Duration::from_secs(600)), 0.0), MAX_WAIT);
    }

    #[test]
    fn parses_retry_after_seconds_only() {
        assert_eq!(parse_retry_after(" 7 "), Some(Duration::from_secs(7)));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2026 07:28:00 GMT"), None);
    }

    #[tokio::test]
    async fn rate_gate_spaces_requests_by_min_interval() {
        let gate = RateGate::new();
        let start = Instant::now();
        gate.wait().await;
        gate.wait().await;
        gate.wait().await;
        // 3 calls => 2 enforced gaps of 500 ms (i.e. at most 2 req/s).
        assert!(start.elapsed() >= Duration::from_millis(1000));
    }
}
