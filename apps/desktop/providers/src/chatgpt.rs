//! Unofficial ChatGPT / Codex subscription provider.
//!
//! This talks to the same private backend as the Codex CLI. It is **not** an
//! official OpenAI API: endpoints, headers and token semantics can change or
//! be blocked without notice. The UI requires an explicit "unofficial, at your
//! own risk" acknowledgement before a row is created.
//!
//! Tokens live only in the existing encrypted secret store (`provider/<id>` /
//! `oauth`). Nothing here logs a token.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context};
use async_trait::async_trait;
use serde_json::Value;

use crate::{
    transport, BillingMode, Provider, ProviderCapabilities, ProviderId, ProviderRequest,
    QuotaSnapshot, StreamBox,
};

mod auth;
mod quota;
mod responses;

pub use auth::*;
pub use quota::normalize_usage;
use quota::rate_limit_exhausted;
pub use responses::request_body;
use responses::{event_stream, truncate};

pub const AUTH_ISSUER: &str = "https://auth.openai.com";
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
pub const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
pub const DEVICE_VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
pub const ORIGINATOR_ENV: &str = "MIX_CODEX_ORIGINATOR";

/// Codex catalog protocol version sent as `client_version`.
///
/// The backend filters the model list by version. MIX-agent's own release
/// number is lower than every current Codex build, so sending it returns an
/// empty catalog. Keep this in sync with the Docker edition; the parity check
/// enforces the shared value.
pub const CODEX_CLIENT_VERSION: &str = "0.166.0";

/// Whether a stored row is the unofficial ChatGPT integration.
///
/// `preset` and `kind` are both user-supplied on the Desktop IPC surface, so
/// every gate must use this helper. Checking only one of them let a caller
/// create `kind: "chatgpt"` with no preset, skipping the consent check and the
/// base-URL lock.
pub fn is_chatgpt(preset: Option<&str>, kind: &str) -> bool {
    preset == Some("chatgpt") || kind == "chatgpt"
}
const DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const REFRESH_SKEW_SECONDS: u64 = 5 * 60;
const QUOTA_TTL_SECONDS: u64 = 60;
const HTTP_TIMEOUT: Duration = Duration::from_secs(45);

pub struct ChatGptProvider {
    id: ProviderId,
    base_url: String,
    session: Arc<ChatGptSession>,
}

impl ChatGptProvider {
    pub fn new(
        id: impl Into<String>,
        base_url: impl Into<String>,
        store: Arc<dyn CredentialStore>,
        raw: Option<&str>,
    ) -> Self {
        let id = id.into();
        let requested = base_url.into();
        // The OAuth token is only ever valid for the Codex backend. Never let a
        // caller-supplied (or legacy/corrupt) row redirect it to another host;
        // pin the host here as well as in the provider CRUD paths.
        if requested.trim_end_matches('/') != BASE_URL {
            tracing::warn!(
                provider = %id,
                %requested,
                "ignoring a non-Codex ChatGPT base URL and using the pinned endpoint"
            );
        }
        let session = ChatGptSession::new(id.clone(), store, raw);
        Self {
            id,
            base_url: BASE_URL.to_string(),
            session,
        }
    }

    /// Adapter with no access to a provider row or secret store.
    ///
    /// `build_provider` cannot construct a working ChatGPT adapter (it has no
    /// provider id or `SecretStore`), so it uses this deliberately inert
    /// variant. Every call reports "ChatGPT login is required"; callers that
    /// can reach storage should use `register_adapter` instead.
    pub fn unauthenticated(base_url: impl Into<String>) -> Self {
        Self::new("", base_url, Arc::new(NoopCredentialStore), None)
    }

    pub fn session(&self) -> Arc<ChatGptSession> {
        self.session.clone()
    }

    /// Fetch the Codex model catalog (`{models: [{slug, display_name, …}]}`).
    pub async fn discover_models(&self) -> anyhow::Result<Vec<crate::discovery::DiscoveredModel>> {
        let mut creds = self.session.token(false).await?;
        let url = format!(
            "{}/models?client_version={}",
            self.base_url.trim_end_matches('/'),
            CODEX_CLIENT_VERSION
        );
        let mut attempts = 0u8;
        let response = loop {
            let response = transport::client()?
                .get(&url)
                .headers(self.session.headers(&creds, None))
                .timeout(Duration::from_secs(20))
                .send()
                .await
                .context("ChatGPT model discovery failed")?;
            self.session.observe_headers(response.headers());
            if attempts == 0 && matches!(response.status().as_u16(), 401 | 403) {
                // The token can be revoked before its advertised expiry. Use
                // the same one-shot refresh as inference instead of making
                // "Connect" fail until some unrelated action rotates it.
                attempts += 1;
                self.session.refresh(true).await?;
                creds = self.session.snapshot().await;
                continue;
            }
            break response;
        };
        if !response.status().is_success() {
            bail!("ChatGPT model discovery returned {}", response.status());
        }
        let body: Value = response
            .json()
            .await
            .context("ChatGPT model discovery returned invalid JSON")?;
        let items = body
            .get("models")
            .or_else(|| body.get("data"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(items
            .into_iter()
            .filter_map(|item| {
                let id = item
                    .get("slug")
                    .or_else(|| item.get("id"))
                    .or_else(|| item.get("name"))
                    .and_then(Value::as_str)?
                    .to_string();
                let label = item
                    .get("display_name")
                    .or_else(|| item.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_string();
                let context_window = item
                    .get("context_window")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .min(u32::MAX as u64) as u32;
                Some(crate::discovery::DiscoveredModel {
                    id,
                    label,
                    context_window,
                    ..Default::default()
                })
            })
            .collect())
    }

    async fn send(
        &self,
        body: &Value,
        creds: &ChatGptCredentials,
        session: &str,
    ) -> anyhow::Result<reqwest::Response> {
        let url = format!("{}/responses", self.base_url.trim_end_matches('/'));
        let request = transport::client()?
            .post(url)
            // The Codex backend expects an SSE accept header. The OpenAI SDK
            // used by the Docker edition sets it automatically; a raw reqwest
            // request does not.
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .headers(self.session.headers(creds, Some(session)))
            .json(body);
        // Retry transport failures and 5xx. A 429 is retried only when the
        // backend did not flag it as an exhausted subscription window; a real
        // quota stop must surface immediately with its reset time.
        let mut attempt = 0u32;
        loop {
            let retry = request
                .try_clone()
                .ok_or_else(|| anyhow::anyhow!("ChatGPT request body is not cloneable"))?;
            match retry.send().await {
                Ok(response) => {
                    let status = response.status();
                    let retryable = status.is_server_error()
                        || (status.as_u16() == 429 && !rate_limit_exhausted(response.headers()));
                    if retryable && attempt < 2 {
                        let wait = crate::reliability::wait_for(
                            attempt,
                            crate::reliability::retry_after_secs(response.headers()),
                        );
                        tracing::warn!(
                            attempt,
                            status = status.as_u16(),
                            wait_ms = wait.as_millis() as u64,
                            "transient ChatGPT response, retrying"
                        );
                        tokio::time::sleep(wait).await;
                        attempt += 1;
                        continue;
                    }
                    return Ok(response);
                }
                Err(err)
                    if crate::reliability::ProviderErrorKind::from_error(&err).is_retryable()
                        && attempt < 2 =>
                {
                    let wait = crate::reliability::wait_for(attempt, None);
                    tracing::warn!(
                        attempt,
                        wait_ms = wait.as_millis() as u64,
                        %err,
                        "transient ChatGPT transport error, retrying"
                    );
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                }
                Err(err) => {
                    return Err(err).context("ChatGPT Responses request failed");
                }
            }
        }
    }
}

#[async_trait]
impl Provider for ChatGptProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn label(&self) -> &str {
        "ChatGPT (Codex)"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            vision: true,
            tool_calling: true,
            reasoning: true,
            streaming: true,
        }
    }

    fn billing(&self) -> BillingMode {
        BillingMode::Subscription
    }

    async fn quota(&self) -> anyhow::Result<Option<QuotaSnapshot>> {
        self.session.quota(false).await
    }

    async fn refresh_quota(&self) -> anyhow::Result<Option<QuotaSnapshot>> {
        self.session.quota(true).await
    }

    async fn stream(&self, req: ProviderRequest) -> anyhow::Result<StreamBox> {
        // `request_body` turns the conversation id into the prompt-cache key;
        // the same value is the provider's `session-id` / `thread-id`. A
        // missing session (probe, extraction) keeps one stable fallback.
        let body = request_body(&req);
        let session = body
            .get("prompt_cache_key")
            .and_then(Value::as_str)
            .unwrap_or("mix-agent")
            .to_string();

        let mut creds = self.session.token(false).await?;
        let mut response = self.send(&body, &creds, &session).await?;
        if matches!(response.status().as_u16(), 401 | 403) {
            self.session.refresh(true).await?;
            creds = self.session.snapshot().await;
            response = self.send(&body, &creds, &session).await?;
            if matches!(response.status().as_u16(), 401 | 403) {
                let status = response.status();
                self.session.mark_auth_failed().await;
                return Err(AuthError::new(
                    true,
                    format!("ChatGPT rejected the refreshed token ({status})"),
                )
                .into());
            }
        }
        // Observe usage headers before classifying the status: a 429 carries
        // the reset time and `limit_reached`, and dropping them left the quota
        // card stale exactly when the user needed it.
        self.session.observe_headers(response.headers());
        if response.status().as_u16() == 429 {
            let reset = response
                .headers()
                .get("x-codex-primary-reset-at")
                .or_else(|| response.headers().get("retry-after"))
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default();
            if rate_limit_exhausted(response.headers()) {
                bail!(
                    "ChatGPT usage limit reached{}",
                    if reset.is_empty() {
                        String::new()
                    } else {
                        format!(" (resets at {reset})")
                    }
                );
            }
            bail!(
                "ChatGPT is rate limiting this account{}",
                if reset.is_empty() {
                    String::new()
                } else {
                    format!(" (retry after {reset})")
                }
            );
        }
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            bail!(
                "ChatGPT Responses returned {status}: {}",
                truncate(&text, 500)
            );
        }
        Ok(event_stream(response, self.session.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base64::Engine;
    use reqwest::header::{HeaderMap, HeaderValue};
    use serde_json::json;

    use super::quota::quota_from_headers;
    use super::*;
    use crate::Message;

    fn request() -> ProviderRequest {
        ProviderRequest {
            model: "gpt-5-codex".into(),
            messages: vec![Message::text("user", "hello")],
            system: Some("be terse".into()),
            tools: Arc::new(Vec::new()),
            temperature: None,
            top_p: None,
            max_tokens: Some(128),
            reasoning: None,
            stream: true,
            session_id: None,
            context_window: None,
            keep_alive: None,
        }
    }

    #[test]
    fn request_body_uses_responses_shape() {
        let body = request_body(&request());
        assert_eq!(body["model"], "gpt-5-codex");
        assert_eq!(body["instructions"], "be terse");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["input"][0]["content"][0]["text"], "hello");
        assert!(body.get("tools").is_none());
        assert!(body["include"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "reasoning.encrypted_content"));
    }

    #[test]
    fn reasoning_off_is_not_silently_upgraded_to_medium() {
        let mut req = request();
        req.reasoning = Some(crate::ReasoningControl::Off);
        let body = request_body(&req);
        assert!(body.get("reasoning").is_none(), "{body}");
    }

    #[test]
    fn xhigh_is_forwarded_as_xhigh() {
        let mut req = request();
        req.reasoning = Some(crate::ReasoningControl::Xhigh);
        let body = request_body(&req);
        assert_eq!(body["reasoning"]["effort"], "xhigh");
    }

    #[test]
    fn the_conversation_id_is_the_prompt_cache_key() {
        let mut req = request();
        req.session_id = Some("conversation-1".into());
        let body = request_body(&req);
        assert_eq!(body["prompt_cache_key"], "mix-conversation-1");
    }

    #[test]
    fn the_fallback_cache_key_is_stable_without_a_session() {
        let first = request_body(&request())["prompt_cache_key"].clone();
        let second = request_body(&request())["prompt_cache_key"].clone();
        assert_eq!(first, second);
        assert!(first.as_str().is_some_and(|key| key.starts_with("mix-")));
    }

    #[test]
    fn native_items_replace_the_reconstructed_assistant_turn() {
        let mut req = request();
        req.messages.push(
            Message::assistant_with_tools("", Vec::new()).with_native(vec![json!({
                "type": "reasoning",
                "encrypted_content": "xyz",
            })]),
        );
        let body = request_body(&req);
        assert_eq!(body["input"][1]["type"], "reasoning");
        assert_eq!(body["input"][1]["encrypted_content"], "xyz");
    }

    #[test]
    fn usage_normalizes_primary_and_secondary() {
        let snapshot = normalize_usage(
            &json!({
                "plan_type": "plus",
                "rate_limit": {
                    "primary_window": {
                        "used_percent": 42,
                        "limit_window_seconds": 18000,
                        "reset_at": 1767225600
                    },
                    "secondary_window": {
                        "used_percent": 120,
                        "limit_window_seconds": 604800,
                        "reset_at": 1767800000
                    }
                }
            }),
            1,
        );
        assert_eq!(snapshot.windows[0].id, "5h");
        assert_eq!(snapshot.windows[0].used_percent, 42.0);
        assert_eq!(snapshot.windows[1].id, "week");
        assert_eq!(snapshot.windows[1].used_percent, 100.0);
    }

    #[test]
    fn a_six_hour_window_is_not_labelled_five_hours() {
        let snapshot = normalize_usage(
            &json!({
                "rate_limit": {
                    "primary_window": {
                        "used_percent": 1,
                        "limit_window_seconds": 6 * 3600
                    }
                }
            }),
            1,
        );
        assert_eq!(snapshot.windows[0].id, "6h");
    }

    #[test]
    fn loose_boolean_flags_are_normalized() {
        for (reached, allowed) in [
            (json!(1), Value::Null),
            (json!("true"), Value::Null),
            (Value::Null, json!("false")),
            (Value::Null, json!(0)),
        ] {
            let snapshot = normalize_usage(
                &json!({
                    "rate_limit": {
                        "limit_reached": reached,
                        "allowed": allowed,
                        "primary_window": {
                            "used_percent": 100,
                            "limit_window_seconds": 18000
                        }
                    }
                }),
                1,
            );
            assert!(snapshot.limit_reached, "{reached} / {allowed}");
        }
    }

    #[test]
    fn credits_accept_loose_booleans_and_numeric_balances() {
        let snapshot = normalize_usage(
            &json!({
                "rate_limit": {
                    "primary_window": {
                        "used_percent": 1,
                        "limit_window_seconds": 18000
                    }
                },
                "credits": { "has_credits": 1, "unlimited": "true", "balance": 12.5 }
            }),
            1,
        );
        let credits = snapshot.credits.expect("credits");
        assert!(credits.has_credits);
        assert!(credits.unlimited);
        assert_eq!(credits.balance, "12.5");
    }

    #[test]
    fn credit_headers_are_case_insensitive() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-codex-primary-used-percent",
            HeaderValue::from_static("1"),
        );
        headers.insert(
            "x-codex-credits-has-credits",
            HeaderValue::from_static("True"),
        );
        headers.insert("x-codex-credits-unlimited", HeaderValue::from_static("1"));
        let snapshot = quota_from_headers(&headers).expect("snapshot");
        assert_eq!(snapshot.windows[0].id, "5h");
        let credits = snapshot.credits.expect("credits");
        assert!(credits.has_credits);
        assert!(credits.unlimited);
    }

    #[test]
    fn allowed_false_counts_as_limit_reached_even_when_the_flag_is_false() {
        let snapshot = normalize_usage(
            &json!({
                "rate_limit": {
                    "allowed": false,
                    "limit_reached": false,
                    "primary_window": {"used_percent": 100, "limit_window_seconds": 18000}
                }
            }),
            1,
        );
        assert!(snapshot.limit_reached);
    }

    #[test]
    fn quota_headers_are_parsed() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-codex-primary-used-percent",
            HeaderValue::from_static("12.5"),
        );
        headers.insert(
            "x-codex-primary-window-minutes",
            HeaderValue::from_static("300"),
        );
        headers.insert(
            "x-codex-primary-reset-at",
            HeaderValue::from_static("1767225600"),
        );
        let snapshot = quota_from_headers(&headers).expect("snapshot");
        assert_eq!(snapshot.windows[0].id, "5h");
        assert_eq!(snapshot.windows[0].used_percent, 12.5);
    }

    #[test]
    fn id_token_claims_are_extracted() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            json!({
                "exp": 1767225600,
                "email": "user@example.com",
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": "acct",
                    "chatgpt_plan_type": "plus"
                }
            })
            .to_string(),
        );
        let token = format!("header.{payload}.sig");
        let (account, plan, email, exp) = parse_id_token(&token);
        assert_eq!(account, "acct");
        assert_eq!(plan, "plus");
        assert_eq!(email, "user@example.com");
        assert_eq!(exp, 1767225600);
    }

    #[test]
    fn system_messages_are_folded_into_instructions() {
        let mut req = request();
        req.messages
            .insert(0, Message::text("system", "<task_state>plan</task_state>"));
        let instructions = request_body(&req)["instructions"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(instructions.contains("be terse"), "{instructions}");
        assert!(
            instructions.contains("<task_state>plan</task_state>"),
            "{instructions}"
        );
    }

    #[test]
    fn orphan_tool_results_are_not_sent() {
        let mut req = request();
        req.messages.push(Message {
            role: "tool".into(),
            content: "orphan output".into(),
            tool_calls: Vec::new(),
            tool_call_id: Some("missing".into()),
            name: Some("read".into()),
            is_error: false,
            native: Vec::new(),
        });
        let body = request_body(&req);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 1);
        assert!(input
            .iter()
            .all(|item| item.get("type").and_then(Value::as_str) != Some("function_call_output")));
    }

    #[test]
    fn usage_uses_relative_reset_and_numeric_balance() {
        let snapshot = normalize_usage(
            &json!({
                "rate_limit": {
                    "primary_window": {
                        "used_percent": 1,
                        "limit_window_seconds": 27000,
                        "reset_after_seconds": 120
                    }
                },
                "credits": { "has_credits": true, "balance": 12.5 }
            }),
            1000,
        );
        assert_eq!(snapshot.windows[0].id, "8h");
        assert_eq!(snapshot.windows[0].resets_at, 1120);
        assert_eq!(snapshot.credits.unwrap().balance, "12.5");
    }

    #[test]
    fn a_failed_login_state_round_trips() {
        let creds = ChatGptCredentials {
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            auth_failed: true,
            ..Default::default()
        };
        let raw = creds.to_secret().unwrap();
        let parsed = ChatGptCredentials::from_secret(Some(&raw));
        assert!(parsed.connected());
        assert!(parsed.auth_failed);
    }

    #[test]
    fn permanent_auth_errors_are_classified() {
        let transient: anyhow::Error = AuthError::new(false, "temporary").into();
        let permanent: anyhow::Error = AuthError::new(true, "sign in again").into();
        assert!(!is_permanent_auth_error(&transient));
        assert!(is_permanent_auth_error(&permanent));
    }

    #[test]
    fn chatgpt_rows_are_detected_by_preset_or_kind() {
        assert!(is_chatgpt(Some("chatgpt"), "openai"));
        assert!(is_chatgpt(None, "chatgpt"));
        assert!(!is_chatgpt(Some("openai"), "openai"));
    }

    #[test]
    fn a_foreign_base_url_cannot_redirect_the_oauth_token() {
        let provider = ChatGptProvider::new(
            "provider-1",
            "https://evil.example/v1",
            Arc::new(NoopCredentialStore),
            None,
        );
        assert_eq!(provider.base_url, BASE_URL);
    }
}
