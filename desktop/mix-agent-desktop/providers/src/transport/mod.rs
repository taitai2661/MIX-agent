//! Shared HTTP transport helpers used by the OpenAI-compatible and Ollama
//! adapters. SSE parsing handles both `data: {...}` and `data: [DONE]` markers.

use std::sync::OnceLock;
use std::time::Duration;

use bytes::Bytes;
use futures_util::stream::{Stream, StreamExt};
use reqwest::{Client, RequestBuilder, Response};
use serde_json::Value;

use crate::reliability::{self, ProviderErrorKind};

/// The process-wide HTTP client.
///
/// Every call site used to build its own `Client`, which rebuilds the
/// connection pool (a TLS handshake per request) and — at most sites — set no
/// timeout, so a host that accepted the connection and then went silent blocked
/// the run *and* its cancellation indefinitely.
///
/// `read_timeout` rather than an overall `timeout`: a total timeout would also
/// cover the body read and kill a long-lived SSE stream, while `read_timeout`
/// only fires when the peer stops sending mid-stream. Callers that need a hard
/// deadline (health probes, token exchange) set `.timeout()` on their request.
pub fn client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(120))
            .pool_max_idle_per_host(4)
            .build()
            .expect("reqwest TLS backend")
    })
}

/// Send a request, retrying transient failures.
///
/// A 429 or 503 used to end the run outright, so a momentary provider overload
/// looked like a crash. Only errors [`ProviderErrorKind`] calls retryable are
/// retried, the wait honours `Retry-After`, and the final error names the
/// status so the caller can classify it without guessing.
pub async fn send_with_retry(request: RequestBuilder) -> anyhow::Result<Response> {
    let mut attempt = 0u32;
    let mut request = request;
    loop {
        let cloned = request.try_clone().ok_or_else(|| {
            anyhow::anyhow!("provider request cannot be retried: the body is not cloneable")
        })?;
        let result = request.send().await;
        let response = match result {
            Ok(response) => response,
            Err(err) => {
                let kind = ProviderErrorKind::from_error(&err);
                if reliability::should_retry(kind, attempt) {
                    let wait = reliability::wait_for(attempt, None);
                    tracing::warn!(
                        attempt,
                        ?kind,
                        wait_ms = wait.as_millis() as u64,
                        %err,
                        "transient provider transport error, retrying"
                    );
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                    request = cloned;
                    continue;
                }
                tracing::error!(attempt, ?kind, %err, "provider request failed");
                return Err(anyhow::anyhow!("provider request failed: {}", err));
            }
        };

        if response.status().is_success() {
            return Ok(response);
        }

        let status = response.status();
        let kind = ProviderErrorKind::from_status(status.as_u16());
        let retry_after = reliability::retry_after_secs(response.headers());
        let body = response.text().await.unwrap_or_default();
        if reliability::should_retry(kind, attempt) {
            let wait = reliability::wait_for(attempt, retry_after);
            tracing::warn!(
                attempt,
                status = status.as_u16(),
                ?kind,
                wait_ms = wait.as_millis() as u64,
                "transient provider status, retrying"
            );
            tokio::time::sleep(wait).await;
            attempt += 1;
            request = cloned;
            continue;
        }
        tracing::error!(
            attempt,
            status = status.as_u16(),
            ?kind,
            body = %truncate(&body),
            "provider rejected the request"
        );
        return Err(anyhow::anyhow!(
            "provider HTTP {} after {} attempt(s): {}",
            status,
            attempt + 1,
            truncate(&body)
        ));
    }
}

/// Build a JSON POST with the headers every provider expects.
pub fn json_post(client: &Client, url: &str, body: &impl serde::Serialize) -> RequestBuilder {
    client
        .post(url)
        .header("content-type", "application/json")
        .json(body)
}

fn truncate(body: &str) -> String {
    const MAX: usize = 800;
    if body.len() <= MAX {
        return body.to_string();
    }
    // Slicing at an arbitrary byte panics when it lands inside a multi-byte
    // character, and an error body in Japanese hits that almost immediately.
    let mut cut = MAX;
    while cut > 0 && !body.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &body[..cut])
}

/// Total time a caller should be willing to wait for a retried request.
pub fn worst_case_duration() -> Duration {
    (0..reliability::MAX_ATTEMPTS)
        .map(reliability::backoff)
        .sum()
}

pub fn sse_stream(
    response: Response,
) -> std::pin::Pin<Box<dyn Stream<Item = anyhow::Result<Bytes>> + Send>> {
    Box::pin(
        response
            .bytes_stream()
            .map(|item| item.map_err(|e| anyhow::anyhow!("SSE read: {}", e))),
    )
}

/// Append a network chunk to the SSE buffer and return every complete event.
///
/// The buffer is raw bytes on purpose: `bytes_stream()` splits the body at
/// arbitrary byte boundaries, so a chunk frequently ends in the middle of a
/// multi-byte character. Decoding each chunk on its own would fail (and used to
/// drop the whole chunk). Only complete lines — which always end on `b'\n'`,
/// never inside a character — are decoded.
pub fn parse_sse_lines(buffer: &mut Vec<u8>, chunk: &Bytes) -> Vec<String> {
    buffer.extend_from_slice(chunk);
    let mut events = Vec::new();
    let mut start = 0usize;
    while let Some(idx) = buffer[start..].iter().position(|&b| b == b'\n') {
        let end = start + idx;
        let raw = &buffer[start..end];
        start = end + 1;
        let line = String::from_utf8_lossy(raw);
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if let Some(data) = line.strip_prefix("data:") {
            events.push(data.trim().to_string());
        }
    }
    if start > 0 {
        buffer.drain(..start);
    }
    events
}

pub fn parse_sse_json<T: serde::de::DeserializeOwned>(data: &str) -> Option<T> {
    if data.is_empty() || data == "[DONE]" {
        return None;
    }
    serde_json::from_str(data).ok()
}

pub fn sse_done(data: &str) -> bool {
    data == "[DONE]"
}

pub fn extract_error(value: &Value) -> Option<String> {
    value
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            value
                .get("error")
                .and_then(|e| e.as_str())
                .map(|s| s.to_string())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reliability::backoff;

    #[test]
    fn error_bodies_are_truncated() {
        let short = "{\"error\": \"nope\"}";
        assert_eq!(truncate(short), short);
        let long = "x".repeat(5000);
        let out = truncate(&long);
        assert!(
            out.len() < 900,
            "a 5 KB error body must not reach the prompt"
        );
        assert!(out.ends_with('…'));
    }

    #[test]
    fn the_worst_case_wait_covers_every_attempt() {
        let total = worst_case_duration();
        assert!(total >= backoff(0));
        assert!(total >= backoff(1));
    }

    #[test]
    fn sse_lines_are_extracted_across_chunk_boundaries() {
        // A JSON payload split mid-line is the normal case for SSE; a parser
        // that only looks within one chunk drops events silently.
        let mut buffer = Vec::new();
        let first = parse_sse_lines(&mut buffer, &Bytes::from_static(b"data: {\"a\""));
        assert!(first.is_empty());
        let second = parse_sse_lines(&mut buffer, &Bytes::from_static(b":1}\ndata: [DONE]\n"));
        assert_eq!(second, vec!["{\"a\":1}".to_string(), "[DONE]".to_string()]);
    }

    #[test]
    fn a_chunk_split_inside_a_multibyte_character_is_not_dropped() {
        // "日" is `e6 97 a5`; a network chunk boundary can land between any two
        // of those bytes. The whole chunk used to be discarded as invalid UTF-8,
        // and the next chunk then started with orphan continuation bytes.
        let payload = "data: {\"text\":\"日本語\"}\n".as_bytes();
        let (head, tail) = payload.split_at(23);
        let mut buffer = Vec::new();
        assert!(parse_sse_lines(&mut buffer, &Bytes::copy_from_slice(head)).is_empty());
        let events = parse_sse_lines(&mut buffer, &Bytes::copy_from_slice(tail));
        assert_eq!(events, vec!["{\"text\":\"日本語\"}".to_string()]);
        assert!(buffer.is_empty());
    }

    #[test]
    fn blank_lines_and_comments_are_ignored() {
        let mut buffer = Vec::new();
        let events = parse_sse_lines(
            &mut buffer,
            &Bytes::from_static(b"\n: keepalive\ndata: x\n"),
        );
        assert_eq!(events, vec!["x".to_string()]);
    }

    #[test]
    fn crlf_line_endings_are_handled() {
        let mut buffer = Vec::new();
        let events = parse_sse_lines(&mut buffer, &Bytes::from_static(b"data: x\r\n"));
        assert_eq!(events, vec!["x".to_string()]);
    }

    #[test]
    fn the_done_marker_is_recognised() {
        assert!(sse_done("[DONE]"));
        assert!(!sse_done("{}"));
        assert!(parse_sse_json::<serde_json::Value>("[DONE]").is_none());
    }

    #[test]
    fn provider_errors_are_extracted_in_both_shapes() {
        let nested = serde_json::json!({"error": {"message": "rate limited"}});
        assert_eq!(extract_error(&nested).as_deref(), Some("rate limited"));
        let flat = serde_json::json!({"error": "bad request"});
        assert_eq!(extract_error(&flat).as_deref(), Some("bad request"));
        assert_eq!(extract_error(&serde_json::json!({})), None);
    }
}
