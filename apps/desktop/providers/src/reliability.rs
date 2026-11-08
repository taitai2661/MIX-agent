//! Provider-side retry policy and error classification.
//!
//! This was an empty module whose doc comment promised retry behaviour that
//! never landed, so a single 429 or 503 failed the entire run: the agent lost
//! all in-memory state and the user had to ask again. Retrying a rate limit is
//! not an optimisation, it is the difference between a run that survives a
//! momentary overload and one that appears to be broken.
//!
//! Two rules keep the retry honest:
//!
//! - only errors that are plausibly transient are retried; a 400 means the
//!   request is wrong and repeating it just burns the budget;
//! - the wait honours `Retry-After` when the server sends one, because the
//!   server knows and the client does not.

use std::time::Duration;

/// How many times a transient failure is retried before the run gives up.
pub const MAX_ATTEMPTS: u32 = 4;
/// Base for the exponential backoff; doubled per attempt.
pub const BASE_BACKOFF_MS: u64 = 500;
/// Never sleep longer than this, whatever the exponent says.
pub const MAX_BACKOFF_MS: u64 = 20_000;

/// What kind of failure this is, and therefore whether retrying can help.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorKind {
    /// Rate limited. Retryable, and `Retry-After` is meaningful.
    RateLimited,
    /// The server is briefly unable to answer. Retryable.
    Unavailable,
    /// The request itself is malformed. Not retryable.
    BadRequest,
    /// Authentication or authorisation failed. Not retryable.
    Unauthorized,
    /// The request exceeded the context or output limit. Not retryable.
    TooLarge,
    /// A network-level failure. Retryable.
    Transport,
    /// Anything else, including a success status with an error body.
    Other,
}

impl ProviderErrorKind {
    /// Stable wire/database name. The Auto router persists this value, so it
    /// must not change when the enum is renamed.
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderErrorKind::RateLimited => "rate_limit",
            ProviderErrorKind::Unavailable => "provider_5xx",
            ProviderErrorKind::BadRequest => "bad_request",
            ProviderErrorKind::Unauthorized => "auth",
            ProviderErrorKind::TooLarge => "context_overflow",
            ProviderErrorKind::Transport => "transport",
            ProviderErrorKind::Other => "other",
        }
    }

    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            ProviderErrorKind::RateLimited
                | ProviderErrorKind::Unavailable
                | ProviderErrorKind::Transport
        )
    }

    /// Whether Auto may retry this failure on a different model before any
    /// content has been emitted.
    ///
    /// Authentication and context-size failures are not transport retries, but
    /// another provider/model can genuinely serve the turn, so they are
    /// eligible for a fallback (the caller still checks that the replacement
    /// has a large enough window).
    pub fn is_fallback_eligible(self) -> bool {
        matches!(
            self,
            ProviderErrorKind::RateLimited
                | ProviderErrorKind::Unavailable
                | ProviderErrorKind::Transport
                | ProviderErrorKind::Unauthorized
                | ProviderErrorKind::TooLarge
        )
    }

    /// Classify from an HTTP status code.
    pub fn from_status(status: u16) -> Self {
        match status {
            408 | 425 => ProviderErrorKind::Unavailable,
            401 | 403 => ProviderErrorKind::Unauthorized,
            429 => ProviderErrorKind::RateLimited,
            400 | 404 | 422 => ProviderErrorKind::BadRequest,
            413 => ProviderErrorKind::TooLarge,
            s if s >= 500 => ProviderErrorKind::Unavailable,
            _ => ProviderErrorKind::Other,
        }
    }

    /// Classify from a transport error message.
    ///
    /// Deliberately conservative: anything not recognised as a connection
    /// problem is `Other`, so an unrecognised failure is surfaced rather than
    /// silently retried five times.
    pub fn from_transport(message: &str) -> Self {
        let lower = message.to_lowercase();
        if lower.contains("timed out")
            || lower.contains("timeout")
            || lower.contains("connection reset")
            || lower.contains("connection refused")
            || lower.contains("broken pipe")
            || lower.contains("dns")
            || lower.contains("eof")
        {
            ProviderErrorKind::Transport
        } else {
            ProviderErrorKind::Other
        }
    }

    /// Classify a real transport error.
    ///
    /// `reqwest::Error`'s `Display` prints only its kind and URL — never the
    /// source chain — so matching on `err.to_string()` classified every
    /// connection reset and refusal as `Other` and disabled retry entirely.
    pub fn from_error(err: &(dyn std::error::Error + 'static)) -> Self {
        let mut current = Some(err);
        while let Some(e) = current {
            if let Some(re) = e.downcast_ref::<reqwest::Error>() {
                if re.is_timeout() || re.is_connect() {
                    return ProviderErrorKind::Transport;
                }
                if let Some(status) = re.status() {
                    return ProviderErrorKind::from_status(status.as_u16());
                }
            }
            let text = e.to_string();
            // `send_with_retry` returns an `anyhow` message that names the
            // final HTTP status; the status is not preserved as a typed source
            // because `reqwest::Error` is consumed. Recover it from the text.
            let from_text = Self::from_message(&text);
            if from_text != ProviderErrorKind::Other {
                return from_text;
            }
            current = e.source();
        }
        ProviderErrorKind::Other
    }

    /// Classify an error string that reached the stream or the run engine
    /// without a typed source.
    ///
    /// Adapters turn SSE `error` payloads and aborted streams into strings, so
    /// this is the last place that can recover the failure taxonomy. It is
    /// intentionally conservative: a status-looking number is only trusted
    /// when it appears as a whole token (`429`, not `1429`), and an
    /// unrecognised message stays `Other` rather than being punished as a
    /// provider outage.
    pub fn from_message(message: &str) -> Self {
        let lower = message.to_lowercase();
        let has = |needle: &str| contains_token(&lower, needle);
        if has("429")
            || lower.contains("rate limit")
            || lower.contains("rate_limit")
            || lower.contains("too many requests")
        {
            ProviderErrorKind::RateLimited
        } else if has("401")
            || has("403")
            || lower.contains("unauthorized")
            || lower.contains("forbidden")
            || lower.contains("invalid api key")
            || lower.contains("incorrect api key")
        {
            ProviderErrorKind::Unauthorized
        } else if has("413")
            || lower.contains("context length")
            || lower.contains("context_length")
            || lower.contains("maximum context")
            || lower.contains("too many tokens")
            || lower.contains("prompt is too long")
        {
            ProviderErrorKind::TooLarge
        } else if lower.contains("timed out")
            || lower.contains("timeout")
            || lower.contains("connection reset")
            || lower.contains("connection refused")
            || lower.contains("broken pipe")
            || lower.contains("dns")
            || lower.contains("unexpected eof")
        {
            ProviderErrorKind::Transport
        } else if has("500")
            || has("502")
            || has("503")
            || has("504")
            || lower.contains("overloaded")
            || lower.contains("service unavailable")
            || lower.contains("internal server error")
        {
            ProviderErrorKind::Unavailable
        } else if has("400")
            || has("404")
            || has("422")
            || lower.contains("invalid request")
            || lower.contains("bad request")
        {
            ProviderErrorKind::BadRequest
        } else {
            ProviderErrorKind::Other
        }
    }
}

/// Whether `needle` appears as a whole token in `haystack`.
///
/// Prevents `1429` or `5030` from being read as HTTP statuses.
fn contains_token(haystack: &str, needle: &str) -> bool {
    let bytes = haystack.as_bytes();
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        let s = start + pos;
        let e = s + needle.len();
        let before = s == 0 || !bytes[s - 1].is_ascii_alphanumeric();
        let after = e >= bytes.len() || !bytes[e].is_ascii_alphanumeric();
        if before && after {
            return true;
        }
        start = e;
    }
    false
}

/// Extract `Retry-After` in seconds, accepting both the delta-seconds and the
/// HTTP-date form. Only the numeric form is honoured; a date form would need a
/// trusted clock and is better treated as absent than as a wrong sleep.
pub fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let raw = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let seconds: u64 = raw.trim().parse().ok()?;
    Some(Duration::from_secs(seconds.min(MAX_BACKOFF_MS / 1000)))
}

/// Backoff for `attempt`, counting from 0. Exponential, then capped.
pub fn backoff(attempt: u32) -> Duration {
    let millis = BASE_BACKOFF_MS.saturating_mul(1u64 << attempt.min(6));
    Duration::from_millis(millis.min(MAX_BACKOFF_MS))
}

/// How long to wait before retrying `attempt` (0-based).
///
/// A `Retry-After` from the server wins over our own schedule, bounded by the
/// same cap so a hostile or mistaken header cannot stall a run for an hour.
pub fn wait_for(attempt: u32, retry_after: Option<Duration>) -> Duration {
    match retry_after {
        Some(server) => server.min(Duration::from_millis(MAX_BACKOFF_MS)),
        None => backoff(attempt),
    }
}

/// Should this failure be retried, given how many attempts have been made?
pub fn should_retry(kind: ProviderErrorKind, attempts_made: u32) -> bool {
    kind.is_retryable() && attempts_made < MAX_ATTEMPTS
}

/// Total attempts allowed, for error messages that should say so.
pub fn max_attempts() -> u32 {
    MAX_ATTEMPTS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_statuses_are_retryable() {
        for status in [408, 425, 429, 500, 502, 503, 504] {
            let kind = ProviderErrorKind::from_status(status);
            assert!(kind.is_retryable(), "status {status} should be retryable");
        }
    }

    #[test]
    fn permanent_statuses_are_not_retryable() {
        // A 400 means the request is wrong; repeating it burns the budget and
        // delays the error the user actually needs to see.
        for status in [400, 401, 403, 404, 413, 422] {
            let kind = ProviderErrorKind::from_status(status);
            assert!(!kind.is_retryable(), "status {status} must not be retried");
        }
    }

    #[test]
    fn an_unrecognised_status_is_not_retried() {
        assert!(!ProviderErrorKind::from_status(302).is_retryable());
    }

    #[test]
    fn network_failures_are_retryable() {
        for message in [
            "connection reset by peer",
            "operation timed out",
            "dns error",
            "broken pipe",
        ] {
            assert!(
                ProviderErrorKind::from_transport(message).is_retryable(),
                "{message} should be retryable"
            );
        }
    }

    #[test]
    fn an_unrecognised_transport_error_is_surfaced_not_retried() {
        assert!(!ProviderErrorKind::from_transport("weird tls state").is_retryable());
    }

    #[test]
    fn retries_stop_at_the_attempt_cap() {
        let kind = ProviderErrorKind::RateLimited;
        for attempt in 0..MAX_ATTEMPTS - 1 {
            assert!(should_retry(kind, attempt), "attempt {attempt}");
        }
        assert!(!should_retry(kind, MAX_ATTEMPTS));
    }

    #[test]
    fn a_permanent_error_is_never_retried_however_many_attempts() {
        for attempt in 0..10 {
            assert!(!should_retry(ProviderErrorKind::BadRequest, attempt));
        }
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        let first = backoff(0);
        let second = backoff(1);
        assert!(second > first, "backoff must grow: {first:?} {second:?}");
        assert!(backoff(20) <= Duration::from_millis(MAX_BACKOFF_MS));
        assert!(backoff(20) > Duration::ZERO);
    }

    #[test]
    fn retry_after_overrides_our_own_schedule() {
        let server = Duration::from_secs(2);
        let wait = wait_for(0, Some(server));
        assert_eq!(wait, server, "the server knows when to come back");
    }

    #[test]
    fn a_hostile_retry_after_is_capped() {
        let wait = wait_for(0, Some(Duration::from_secs(3600)));
        assert!(
            wait <= Duration::from_millis(MAX_BACKOFF_MS),
            "a mistaken header must not stall the run: {wait:?}"
        );
    }

    #[test]
    fn reads_the_numeric_retry_after_header() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("retry-after", "3".parse().unwrap());
        assert_eq!(retry_after_secs(&headers), Some(Duration::from_secs(3)));
    }

    #[test]
    fn an_http_date_retry_after_is_ignored() {
        // Honouring it would need a trusted clock; guessing is worse than
        // falling back to our own backoff.
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "retry-after",
            "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap(),
        );
        assert_eq!(retry_after_secs(&headers), None);
        assert_eq!(wait_for(0, retry_after_secs(&headers)), backoff(0));
    }

    #[test]
    fn a_missing_retry_after_falls_back_to_backoff() {
        let headers = reqwest::header::HeaderMap::new();
        assert_eq!(retry_after_secs(&headers), None);
        assert_eq!(wait_for(1, None), backoff(1));
    }

    #[test]
    fn messages_are_classified_by_status_and_phrase() {
        assert_eq!(
            ProviderErrorKind::from_message("provider HTTP 429 after 4 attempts"),
            ProviderErrorKind::RateLimited
        );
        assert_eq!(
            ProviderErrorKind::from_message("503 Service Unavailable"),
            ProviderErrorKind::Unavailable
        );
        assert_eq!(
            ProviderErrorKind::from_message("invalid api key"),
            ProviderErrorKind::Unauthorized
        );
        assert_eq!(
            ProviderErrorKind::from_message("context length exceeded"),
            ProviderErrorKind::TooLarge
        );
        assert_eq!(
            ProviderErrorKind::from_message("request timed out"),
            ProviderErrorKind::Transport
        );
        assert_eq!(
            ProviderErrorKind::from_message(
                "the provider stream ended without a completion signal"
            ),
            ProviderErrorKind::Other
        );
    }

    #[test]
    fn status_like_substrings_are_not_mistaken_for_http_codes() {
        // 1429 and 5030 are not statuses; the token boundary keeps them from
        // poisoning a model's reliability score.
        assert_eq!(
            ProviderErrorKind::from_message("usage 1429 tokens"),
            ProviderErrorKind::Other
        );
        assert_eq!(
            ProviderErrorKind::from_message("model 5030 in catalogue"),
            ProviderErrorKind::Other
        );
    }

    #[test]
    fn database_names_are_stable() {
        assert_eq!(ProviderErrorKind::RateLimited.as_str(), "rate_limit");
        assert_eq!(ProviderErrorKind::Unavailable.as_str(), "provider_5xx");
        assert_eq!(ProviderErrorKind::Unauthorized.as_str(), "auth");
        assert_eq!(ProviderErrorKind::TooLarge.as_str(), "context_overflow");
        assert_eq!(ProviderErrorKind::Transport.as_str(), "transport");
    }

    #[test]
    fn a_status_named_only_in_an_anyhow_message_is_recovered() {
        let err = anyhow::anyhow!("provider HTTP 503 after 4 attempt(s): overloaded");
        assert_eq!(
            ProviderErrorKind::from_error(err.as_ref()),
            ProviderErrorKind::Unavailable
        );
        let err = anyhow::anyhow!("provider HTTP 429 after 4 attempt(s)");
        assert_eq!(
            ProviderErrorKind::from_error(err.as_ref()),
            ProviderErrorKind::RateLimited
        );
    }

    #[test]
    fn only_recoverable_failures_are_fallback_eligible() {
        for kind in [
            ProviderErrorKind::RateLimited,
            ProviderErrorKind::Unavailable,
            ProviderErrorKind::Transport,
            ProviderErrorKind::Unauthorized,
            ProviderErrorKind::TooLarge,
        ] {
            assert!(kind.is_fallback_eligible(), "{kind:?} should fall back");
        }
        for kind in [ProviderErrorKind::BadRequest, ProviderErrorKind::Other] {
            assert!(
                !kind.is_fallback_eligible(),
                "{kind:?} must not trigger a fallback"
            );
        }
    }
}
