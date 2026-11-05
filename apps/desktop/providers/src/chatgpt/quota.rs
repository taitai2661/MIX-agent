//! ChatGPT / Codex quota normalization.
//!
//! Kept separate from the auth and Responses code so the `/wham/usage`
//! payload, the `x-codex-*` headers and the `codex.rate_limits` SSE event are
//! normalized by one testable set of functions.

use std::time::Duration;

use anyhow::{bail, Context};
use reqwest::header::HeaderMap;
use serde_json::{json, Value};

use super::auth::{auth_headers, now_secs, AuthError, ChatGptCredentials};
use super::USAGE_URL;
use crate::{transport, QuotaCredits, QuotaSnapshot, QuotaWindow};

pub(super) async fn fetch_usage(creds: &ChatGptCredentials) -> anyhow::Result<QuotaSnapshot> {
    let response = transport::client()?
        .get(USAGE_URL)
        .headers(auth_headers(creds, None))
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .context("ChatGPT usage request failed")?;
    let status = response.status();
    if matches!(status.as_u16(), 401 | 403) {
        // Keep this typed so the caller can refresh once and retry, just like
        // inference does after a rejected token.
        return Err(AuthError::new(false, format!("ChatGPT usage returned {status}")).into());
    }
    if !status.is_success() {
        bail!("ChatGPT usage returned {status}");
    }
    let payload: Value = response
        .json()
        .await
        .context("ChatGPT usage returned invalid JSON")?;
    Ok(normalize_usage(&payload, now_secs()))
}

fn window_id(seconds: u64, fallback: &str) -> String {
    if seconds == 0 {
        return fallback.to_string();
    }
    // Exactly six hours used to be labelled as the five-hour window. Leave
    // the boundary itself to the rounded branch below.
    if seconds < 6 * 3600 {
        "5h".to_string()
    } else if seconds >= 6 * 86400 {
        "week".to_string()
    } else {
        // Round to the nearest hour so a 27000-second window reads as 8h, not
        // the 7h that floor division produced.
        format!("{}h", ((seconds + 1800) / 3600).max(1))
    }
}

fn as_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        _ => String::new(),
    }
}

fn as_u64(value: Option<&Value>) -> u64 {
    value
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0)
}

fn as_f64(value: Option<&Value>) -> f64 {
    value
        .and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0.0)
}

/// Normalize the loose booleans the private backend emits (`true`, `1`,
/// `"true"`, …). The old `as_bool()` check silently missed the non-bool forms.
fn as_loose_bool(value: Option<&Value>) -> Option<bool> {
    let value = value?;
    if let Some(flag) = value.as_bool() {
        return Some(flag);
    }
    if let Some(number) = value.as_u64() {
        return Some(number != 0);
    }
    if let Some(number) = value.as_f64() {
        return Some(number != 0.0);
    }
    if let Some(text) = value.as_str() {
        return match text.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" | "" => Some(false),
            _ => None,
        };
    }
    None
}

/// Normalize `/wham/usage` or a `codex.rate_limits` event.
pub fn normalize_usage(payload: &Value, captured_at: u64) -> QuotaSnapshot {
    let rate = payload.get("rate_limit").unwrap_or(&Value::Null);
    let mut windows = Vec::new();
    for (key, fallback) in [("primary_window", "5h"), ("secondary_window", "week")] {
        let Some(window) = rate.get(key).filter(|v| v.is_object()) else {
            continue;
        };
        let mut seconds = as_u64(window.get("limit_window_seconds"));
        if seconds == 0 {
            seconds = as_u64(window.get("window_minutes")) * 60;
        }
        let reset_at = {
            let absolute = as_u64(window.get("reset_at"));
            if absolute != 0 {
                absolute
            } else {
                // Some snapshots only say how long is left.
                captured_at.saturating_add(as_u64(window.get("reset_after_seconds")))
            }
        };
        windows.push(QuotaWindow {
            id: window_id(seconds, fallback),
            used_percent: as_f64(window.get("used_percent")).clamp(0.0, 100.0),
            // Half-up, matching Python's `(seconds + 30) // 60`.
            window_minutes: (seconds + 30) / 60,
            window_seconds: seconds,
            resets_at: reset_at,
        });
    }
    let credits = payload
        .get("credits")
        .filter(|v| v.is_object())
        .map(|credits| QuotaCredits {
            // The same backend has emitted `true`, `1` and `"true"` here, so
            // use the loose bool helper rather than `as_bool`.
            has_credits: as_loose_bool(credits.get("has_credits")) == Some(true),
            unlimited: as_loose_bool(credits.get("unlimited")) == Some(true),
            balance: as_text(credits.get("balance")),
        });
    QuotaSnapshot {
        plan_type: payload
            .get("plan_type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        captured_at,
        windows,
        credits,
        // `allowed: false` means the limit was reached even when the payload
        // carries an explicit `limit_reached: false` (Python treated that pair
        // as reached; keep the editions identical).
        limit_reached: as_loose_bool(rate.get("limit_reached")) == Some(true)
            || as_loose_bool(rate.get("allowed")) == Some(false),
    }
}

fn header_flag(headers: &HeaderMap, name: &str) -> bool {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Whether a rate-limit signal means the subscription window is exhausted.
pub(super) fn rate_limit_exhausted(headers: &HeaderMap) -> bool {
    headers
        .get("x-codex-rate-limit-reached-type")
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !value.is_empty() && !matches!(value.as_str(), "none" | "false" | "0")
        })
        .unwrap_or(false)
}

pub(super) fn quota_from_headers(headers: &HeaderMap) -> Option<QuotaSnapshot> {
    let mut windows = Vec::new();
    for (suffix, fallback) in [("primary", "5h"), ("secondary", "week")] {
        let used = headers
            .get(format!("x-codex-{suffix}-used-percent"))
            .and_then(|v| v.to_str().ok());
        let Some(used) = used else { continue };
        let minutes = headers
            .get(format!("x-codex-{suffix}-window-minutes"))
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let reset_at = headers
            .get(format!("x-codex-{suffix}-reset-at"))
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        windows.push(QuotaWindow {
            id: window_id(minutes * 60, fallback),
            used_percent: used.parse::<f64>().unwrap_or(0.0).clamp(0.0, 100.0),
            window_minutes: minutes,
            window_seconds: minutes * 60,
            resets_at: reset_at,
        });
    }
    if windows.is_empty() {
        return None;
    }
    let credits = headers
        .get("x-codex-credits-has-credits")
        .and_then(|v| v.to_str().ok())
        .map(|_| QuotaCredits {
            has_credits: header_flag(headers, "x-codex-credits-has-credits"),
            unlimited: header_flag(headers, "x-codex-credits-unlimited"),
            balance: headers
                .get("x-codex-credits-balance")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string(),
        });
    Some(QuotaSnapshot {
        plan_type: String::new(),
        captured_at: now_secs(),
        windows,
        credits,
        limit_reached: rate_limit_exhausted(headers),
    })
}

pub(super) fn quota_from_event(value: &Value) -> Option<QuotaSnapshot> {
    if value.get("type").and_then(Value::as_str) != Some("codex.rate_limits") {
        return None;
    }
    let details = value.get("rate_limits")?;
    let payload = json!({
        "plan_type": value.get("plan_type").cloned().unwrap_or(Value::Null),
        "rate_limit": {
            "primary_window": details.get("primary").cloned().unwrap_or(Value::Null),
            "secondary_window": details.get("secondary").cloned().unwrap_or(Value::Null),
            "limit_reached": details.get("limit_reached").cloned().unwrap_or(Value::Null),
        },
        "credits": value.get("credits").cloned().unwrap_or(Value::Null),
    });
    let snapshot = normalize_usage(&payload, now_secs());
    (!snapshot.windows.is_empty()).then_some(snapshot)
}
