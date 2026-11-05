//! ChatGPT / Codex credentials, OAuth device flow, token refresh and quota
//! session state.
//!
//! Tokens are only ever persisted through the encrypted `SecretStore`; this
//! module never logs their contents.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context};
use base64::Engine;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};

use super::quota::{fetch_usage, quota_from_event, quota_from_headers};
use super::responses::truncate;
use super::{
    AUTH_ISSUER, CLIENT_ID, DEVICE_REDIRECT_URI, DEVICE_VERIFICATION_URL, HTTP_TIMEOUT,
    ORIGINATOR_ENV, QUOTA_TTL_SECONDS, REFRESH_SKEW_SECONDS,
};
use crate::{transport, QuotaSnapshot};

/// Persistence seam for ChatGPT OAuth credentials.
///
/// The provider crate deliberately does not depend on the desktop storage
/// layer. The app supplies an implementation backed by the existing encrypted
/// `provider/<id>` / `oauth` secret slot; tests use an in-memory store.
#[async_trait::async_trait]
pub trait CredentialStore: Send + Sync {
    async fn load(&self) -> anyhow::Result<Option<String>>;
    async fn save(&self, raw: &str) -> anyhow::Result<()>;
}

/// No-op store for the inert adapter and wire-only tests.
#[derive(Debug, Default)]
pub struct NoopCredentialStore;

#[async_trait::async_trait]
impl CredentialStore for NoopCredentialStore {
    async fn load(&self) -> anyhow::Result<Option<String>> {
        Ok(None)
    }

    async fn save(&self, _raw: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

/// A token refresh failure. `permanent` distinguishes "the refresh token was
/// rejected, sign in again" from a transient network/5xx error, so the caller
/// can mark the provider as needing a new login instead of silently retrying.
#[derive(Debug)]
pub struct AuthError {
    pub permanent: bool,
    message: String,
}

impl AuthError {
    pub(super) fn new(permanent: bool, message: impl Into<String>) -> Self {
        Self {
            permanent,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AuthError {}

/// Whether an error from this module means the stored login can no longer be
/// used without a fresh device-code authorization.
pub fn is_permanent_auth_error(err: &anyhow::Error) -> bool {
    err.downcast_ref::<AuthError>()
        .is_some_and(|err| err.permanent)
}

/// Whether an error came from the auth layer at all (permanent or transient).
///
/// Quota reads use this to refresh once after a 401/403, matching what
/// inference already does, without retrying ordinary HTTP failures.
pub fn is_auth_error(err: &anyhow::Error) -> bool {
    err.downcast_ref::<AuthError>().is_some()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatGptCredentials {
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub id_token: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub plan_type: String,
    #[serde(default)]
    pub expires_at: u64,
    #[serde(default)]
    pub last_refresh: String,
    /// Set when the refresh endpoint rejected the stored refresh token. The
    /// access token may still work until it expires, but the UI stops showing
    /// the account as fully connected and offers "sign in again".
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auth_failed: bool,
}

impl ChatGptCredentials {
    pub fn from_secret(raw: Option<&str>) -> Self {
        let Some(raw) = raw.filter(|raw| !raw.trim().is_empty()) else {
            return Self::default();
        };
        match serde_json::from_str(raw) {
            Ok(creds) => creds,
            Err(err) => {
                // A corrupted / legacy blob used to look exactly like "not
                // signed in", which made a real store problem invisible. Warn
                // once per process so a listing loop does not spam the log.
                static WARNED: OnceLock<()> = OnceLock::new();
                if WARNED.set(()).is_ok() {
                    tracing::warn!(
                        %err,
                        "stored ChatGPT credentials are unreadable; treating the provider as signed out"
                    );
                }
                Self::default()
            }
        }
    }

    pub fn to_secret(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    pub fn connected(&self) -> bool {
        !self.access_token.is_empty() || !self.refresh_token.is_empty()
    }

    pub fn needs_refresh(&self, now: u64) -> bool {
        self.access_token.is_empty()
            || self.expires_at == 0
            || self.expires_at.saturating_sub(REFRESH_SKEW_SECONDS) <= now
    }

    pub fn public_account(&self) -> Value {
        json!({
            "account_id": self.account_id,
            "email": self.email,
            "plan_type": self.plan_type,
        })
    }
}

/// Whether a token another session persisted is newer than the one in memory.
///
/// Adopt even when the persisted token is already inside the refresh skew: the
/// refresh token was rotated, so refreshing with the previous one is rejected
/// and logs the user out.
fn persisted_supersedes(current: &ChatGptCredentials, persisted: &ChatGptCredentials) -> bool {
    persisted.connected()
        && (persisted.access_token != current.access_token
            || persisted.refresh_token != current.refresh_token)
}

pub(super) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn originator() -> String {
    std::env::var(ORIGINATOR_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "mix_agent".to_string())
}

fn jwt_payload(jwt: &str) -> Value {
    let mut parts = jwt.split('.');
    let (Some(_header), Some(payload), Some(_sig)) = (parts.next(), parts.next(), parts.next())
    else {
        return Value::Null;
    };
    let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload) else {
        return Value::Null;
    };
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// Parse the identity claims Codex stores under `https://api.openai.com/auth`.
pub fn parse_id_token(jwt: &str) -> (String, String, String, u64) {
    let payload = jwt_payload(jwt);
    let auth = payload
        .get("https://api.openai.com/auth")
        .cloned()
        .unwrap_or(Value::Null);
    let account_id = auth
        .get("chatgpt_account_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let plan_type = auth
        .get("chatgpt_plan_type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let email = payload
        .get("email")
        .and_then(Value::as_str)
        .or_else(|| payload.pointer("/profile/email").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    let expires_at = payload
        .get("exp")
        .and_then(Value::as_f64)
        .map(|v| v.max(0.0) as u64)
        .unwrap_or(0);
    (account_id, plan_type, email, expires_at)
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    id_token: String,
}

fn credentials_from_tokens(tokens: TokenResponse) -> ChatGptCredentials {
    let (account_id, plan_type, email, expires_at) = parse_id_token(&tokens.id_token);
    ChatGptCredentials {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        id_token: tokens.id_token,
        account_id,
        plan_type,
        email,
        expires_at: if expires_at == 0 {
            now_secs() + 3600
        } else {
            expires_at
        },
        last_refresh: chrono::Utc::now().to_rfc3339(),
        auth_failed: false,
    }
}

/// Classify a rejected refresh attempt.
///
/// Match Docker's behaviour: 401/403 and the explicit OAuth error codes are
/// permanent, while an unrecognised 400/5xx may be transient. The old "any 400
/// is permanent" rule logged users out for a malformed or proxy-generated
/// request.
fn classify_refresh_failure(status: reqwest::StatusCode, body: &str) -> AuthError {
    let code = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .or_else(|| value.get("code"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    let permanent = matches!(status.as_u16(), 401 | 403)
        || matches!(
            code.as_str(),
            "invalid_grant"
                | "refresh_token_expired"
                | "refresh_token_reused"
                | "refresh_token_invalidated"
        )
        || body.contains("refresh_token_invalidated")
        || body.contains("invalid_grant");
    AuthError::new(
        permanent,
        format!(
            "ChatGPT token refresh failed ({status}){}: {}",
            if permanent { " — sign in again" } else { "" },
            truncate(body, 300)
        ),
    )
}

async fn refresh_credentials(creds: &ChatGptCredentials) -> anyhow::Result<ChatGptCredentials> {
    if creds.refresh_token.is_empty() {
        return Err(AuthError::new(true, "ChatGPT login is required (no refresh token)").into());
    }
    let body = json!({
        "grant_type": "refresh_token",
        "client_id": CLIENT_ID,
        "refresh_token": creds.refresh_token,
        "refresh_reason": "proactive_expiry",
    });
    let response = transport::client()?
        .post(format!("{AUTH_ISSUER}/oauth/token"))
        .json(&body)
        .timeout(HTTP_TIMEOUT)
        .send()
        .await
        .context("ChatGPT token refresh request failed")?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(classify_refresh_failure(status, &body).into());
    }
    let tokens: TokenResponse = response
        .json()
        .await
        .context("ChatGPT token refresh returned invalid JSON")?;
    let mut refreshed = credentials_from_tokens(tokens);
    refreshed.refresh_token = if refreshed.refresh_token.is_empty() {
        creds.refresh_token.clone()
    } else {
        refreshed.refresh_token
    };
    refreshed.id_token = if refreshed.id_token.is_empty() {
        creds.id_token.clone()
    } else {
        refreshed.id_token
    };
    refreshed.account_id = if refreshed.account_id.is_empty() {
        creds.account_id.clone()
    } else {
        refreshed.account_id
    };
    refreshed.email = if refreshed.email.is_empty() {
        creds.email.clone()
    } else {
        refreshed.email
    };
    refreshed.plan_type = if refreshed.plan_type.is_empty() {
        creds.plan_type.clone()
    } else {
        refreshed.plan_type
    };
    Ok(refreshed)
}

pub async fn revoke_credentials(creds: &ChatGptCredentials) -> anyhow::Result<()> {
    let token = if !creds.refresh_token.is_empty() {
        &creds.refresh_token
    } else {
        &creds.access_token
    };
    if token.is_empty() {
        return Ok(());
    }
    let mut body = json!({
        "token": token,
        "token_type_hint": if creds.refresh_token.is_empty() { "access_token" } else { "refresh_token" },
    });
    if !creds.refresh_token.is_empty() {
        body["client_id"] = json!(CLIENT_ID);
    }
    let _ = transport::client()?
        .post(format!("{AUTH_ISSUER}/oauth/revoke"))
        .json(&body)
        .timeout(Duration::from_secs(10))
        .send()
        .await;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct DeviceCodeStart {
    pub device_auth_id: String,
    pub user_code: String,
    pub interval: u64,
    pub verification_url: String,
}

#[derive(Debug, Deserialize)]
struct DeviceUserCode {
    device_auth_id: String,
    #[serde(alias = "usercode")]
    user_code: String,
    #[serde(default)]
    interval: Value,
}

fn device_interval(value: &Value) -> u64 {
    match value {
        Value::String(text) => text.trim().parse().unwrap_or(5),
        Value::Number(number) => number.as_u64().unwrap_or(5),
        _ => 5,
    }
    .max(2)
}

pub async fn request_device_code() -> anyhow::Result<DeviceCodeStart> {
    let response = transport::client()?
        .post(format!("{AUTH_ISSUER}/api/accounts/deviceauth/usercode"))
        .json(&json!({ "client_id": CLIENT_ID }))
        .header("originator", originator())
        .timeout(HTTP_TIMEOUT)
        .send()
        .await
        .context("ChatGPT device-code request failed")?;
    if response.status().as_u16() == 404 {
        bail!("ChatGPT device-code login is not enabled for this account");
    }
    if !response.status().is_success() {
        bail!("ChatGPT device-code request returned {}", response.status());
    }
    let body: DeviceUserCode = response
        .json()
        .await
        .context("ChatGPT device-code returned invalid JSON")?;
    Ok(DeviceCodeStart {
        device_auth_id: body.device_auth_id,
        user_code: body.user_code,
        interval: device_interval(&body.interval),
        verification_url: DEVICE_VERIFICATION_URL.to_string(),
    })
}

#[derive(Debug, Deserialize)]
struct DeviceAuthorization {
    authorization_code: String,
    code_verifier: String,
}

/// Poll once. `Ok(None)` means the user has not authorized yet.
pub async fn poll_device_code(
    device_auth_id: &str,
    user_code: &str,
) -> anyhow::Result<Option<ChatGptCredentials>> {
    let client = transport::client()
        .map_err(|err| AuthError::new(false, format!("ChatGPT device-code poll failed: {err}")))?;
    let response = client
        .post(format!("{AUTH_ISSUER}/api/accounts/deviceauth/token"))
        .json(&json!({
            "device_auth_id": device_auth_id,
            "user_code": user_code,
        }))
        .header("originator", originator())
        .timeout(HTTP_TIMEOUT)
        .send()
        .await
        .map_err(|err| AuthError::new(false, format!("ChatGPT device-code poll failed: {err}")))?;
    if matches!(response.status().as_u16(), 403 | 404) {
        // Codex uses 403/404 for "not authorized yet", but the same statuses
        // are also used for an expired one-time code. Peek at the body so an
        // expired code fails immediately instead of looking pending until the
        // local 15-minute TTL runs out.
        let body = response
            .text()
            .await
            .unwrap_or_default()
            .to_ascii_lowercase();
        if body.contains("expired")
            || body.contains("invalid_grant")
            || body.contains("device_code_expired")
        {
            return Err(
                AuthError::new(true, "ChatGPT device code expired; start the login again").into(),
            );
        }
        return Ok(None);
    }
    if !response.status().is_success() {
        let status = response.status();
        // A 5xx (or an explicit throttling/retry status) is usually the auth
        // service having a bad minute; keep the code alive and let the caller
        // back off. Other 4xx responses are terminal.
        let transient = status.is_server_error() || matches!(status.as_u16(), 408 | 425 | 429);
        return Err(AuthError::new(
            !transient,
            format!("ChatGPT device-code poll returned {status}"),
        )
        .into());
    }
    let code: DeviceAuthorization = response
        .json()
        .await
        .context("ChatGPT device-code poll returned invalid JSON")?;
    exchange_authorization_code(&code.authorization_code, &code.code_verifier).await
}

pub async fn exchange_authorization_code(
    code: &str,
    code_verifier: &str,
) -> anyhow::Result<Option<ChatGptCredentials>> {
    let form = [
        ("grant_type", "authorization_code"),
        ("client_id", CLIENT_ID),
        ("code", code),
        ("redirect_uri", DEVICE_REDIRECT_URI),
        ("code_verifier", code_verifier),
    ];
    let client = transport::client().map_err(|err| {
        AuthError::new(
            false,
            format!("ChatGPT authorization-code exchange failed: {err}"),
        )
    })?;
    let response = client
        .post(format!("{AUTH_ISSUER}/oauth/token"))
        .form(&form)
        .timeout(HTTP_TIMEOUT)
        .send()
        .await
        .map_err(|err| {
            AuthError::new(
                false,
                format!("ChatGPT authorization-code exchange failed: {err}"),
            )
        })?;
    if !response.status().is_success() {
        let status = response.status();
        return Err(AuthError::new(
            !status.is_server_error(),
            format!("ChatGPT authorization-code exchange returned {status}"),
        )
        .into());
    }
    let tokens: TokenResponse = response
        .json()
        .await
        .context("ChatGPT authorization-code exchange returned invalid JSON")?;
    Ok(Some(credentials_from_tokens(tokens)))
}

/// Process-wide refresh locks, keyed by provider row.
///
/// A `ChatGptSession` is created for each adapter instance, and the app creates
/// short-lived adapters for model discovery and quota reads. A per-session lock
/// therefore let two sessions replay the same refresh token at once; because
/// OpenAI rotates refresh tokens, the loser was logged out. The map is tiny
/// (one entry per provider) and never needs eviction in a desktop process.
fn refresh_lock_for(key: &str) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .entry(key.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// Refreshable credentials for one provider row, shared by all turns.
pub struct ChatGptSession {
    provider_id: String,
    store: Arc<dyn CredentialStore>,
    credentials: RwLock<ChatGptCredentials>,
    /// Cached quota snapshot, updated synchronously from response headers and
    /// SSE events. A `std` mutex keeps those updates ordered: a detached task
    /// could otherwise land an older snapshot after a newer one.
    quota: Arc<StdMutex<Option<(u64, QuotaSnapshot)>>>,
}

impl ChatGptSession {
    pub fn new(
        provider_id: impl Into<String>,
        store: Arc<dyn CredentialStore>,
        raw: Option<&str>,
    ) -> Arc<Self> {
        Arc::new(Self {
            provider_id: provider_id.into(),
            store,
            credentials: RwLock::new(ChatGptCredentials::from_secret(raw)),
            quota: Arc::new(StdMutex::new(None)),
        })
    }

    pub async fn snapshot(&self) -> ChatGptCredentials {
        self.credentials.read().await.clone()
    }

    async fn persist(&self, creds: &ChatGptCredentials) -> anyhow::Result<()> {
        let raw = creds.to_secret()?;
        let mut last: Option<anyhow::Error> = None;
        for attempt in 0..3 {
            match self.store.save(&raw).await {
                Ok(()) => return Ok(()),
                Err(err) => {
                    tracing::error!(
                        %err,
                        provider = %self.provider_id,
                        attempt = attempt + 1,
                        "could not persist refreshed ChatGPT credentials"
                    );
                    last = Some(err);
                    tokio::time::sleep(Duration::from_millis(50 * (attempt + 1))).await;
                }
            }
        }
        Err(last.unwrap_or_else(|| anyhow::anyhow!("could not persist ChatGPT credentials")))
    }

    pub async fn token(&self, force_refresh: bool) -> anyhow::Result<ChatGptCredentials> {
        let current = self.snapshot().await;
        if !current.connected() {
            return Err(AuthError::new(true, "ChatGPT login is required").into());
        }
        let needs_refresh = current.needs_refresh(now_secs());
        // A rejected refresh token must not be replayed on every turn. A still
        // valid access token remains usable; only require a fresh login when
        // the token would have to be renewed. The post-401 path passes
        // `force_refresh` because the server just rejected the access token.
        if current.auth_failed && needs_refresh && !force_refresh {
            return Err(AuthError::new(true, "ChatGPT login must be renewed").into());
        }
        if force_refresh || needs_refresh {
            self.refresh(force_refresh).await?;
        }
        let creds = self.snapshot().await;
        if creds.access_token.is_empty() {
            return Err(AuthError::new(true, "ChatGPT login is required").into());
        }
        Ok(creds)
    }

    /// Refresh the access token. `force` bypasses the expiry check (used after
    /// a 401 response), but still adopts a token another session just saved.
    pub async fn refresh(&self, force: bool) -> anyhow::Result<()> {
        let key = {
            let current = self.snapshot().await;
            if !self.provider_id.is_empty() {
                self.provider_id.clone()
            } else if !current.account_id.is_empty() {
                current.account_id.clone()
            } else {
                "chatgpt".to_string()
            }
        };
        let lock = refresh_lock_for(&key);
        let _guard = lock.lock().await;

        let mut current = self.snapshot().await;
        // Another adapter for the same row may have refreshed while this task
        // waited for the lock. Prefer the persisted token over replaying a
        // refresh token that has already been rotated.
        if let Ok(Some(raw)) = self.store.load().await {
            let persisted = ChatGptCredentials::from_secret(Some(&raw));
            if persisted_supersedes(&current, &persisted) {
                *self.credentials.write().await = persisted.clone();
                if !persisted.needs_refresh(now_secs()) {
                    return Ok(());
                }
                current = persisted;
            }
        }

        // A permanently rejected refresh token stays rejected until the user
        // signs in again. `force` is the one-shot post-401 retry and is
        // allowed through so a transient failure can recover.
        if current.auth_failed && !force {
            return Err(AuthError::new(true, "ChatGPT login must be renewed").into());
        }

        if !force && !current.needs_refresh(now_secs()) {
            return Ok(());
        }

        match refresh_credentials(&current).await {
            Ok(refreshed) => {
                *self.credentials.write().await = refreshed.clone();
                // A rotated refresh token that never reached the store is
                // lost across a restart. Surface the failure instead of
                // pretending the session is durable.
                self.persist(&refreshed).await.map_err(|err| {
                    err.context("the refreshed ChatGPT credentials could not be saved")
                })?;
                Ok(())
            }
            Err(err) => {
                if is_permanent_auth_error(&err) {
                    let mut failed = current;
                    failed.auth_failed = true;
                    *self.credentials.write().await = failed.clone();
                    if let Err(persist_err) = self.persist(&failed).await {
                        tracing::error!(%persist_err, provider = %self.provider_id, "could not persist the failed ChatGPT login state");
                    }
                }
                Err(err)
            }
        }
    }

    /// Record that the server rejected a freshly refreshed access token.
    ///
    /// A second 401/403 after a forced refresh means the stored login can no
    /// longer be used; persist the flag so the UI offers "sign in again"
    /// instead of retrying the same rejected token every turn.
    pub async fn mark_auth_failed(&self) {
        let mut failed = self.snapshot().await;
        failed.auth_failed = true;
        *self.credentials.write().await = failed.clone();
        if let Err(err) = self.persist(&failed).await {
            tracing::error!(
                %err,
                provider = %self.provider_id,
                "could not persist the rejected ChatGPT login state"
            );
        }
    }

    pub fn headers(&self, creds: &ChatGptCredentials, session: Option<&str>) -> HeaderMap {
        auth_headers(creds, session)
    }

    fn cached_quota(&self) -> Option<(u64, QuotaSnapshot)> {
        self.quota
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn store_quota(&self, snapshot: QuotaSnapshot) {
        let mut guard = self
            .quota
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard
            .as_ref()
            .is_none_or(|(_, current)| snapshot.captured_at >= current.captured_at)
        {
            *guard = Some((now_secs(), snapshot));
        }
    }

    pub async fn quota(&self, force: bool) -> anyhow::Result<Option<QuotaSnapshot>> {
        let cached = self.cached_quota();
        if let Some((at, snapshot)) = &cached {
            if !force && now_secs().saturating_sub(*at) < QUOTA_TTL_SECONDS {
                return Ok(Some(snapshot.clone()));
            }
        }

        let snapshot = match self.fetch_quota_once().await {
            Ok(snapshot) => snapshot,
            // A token can be rejected before its advertised expiry (clock skew,
            // revocation, a rotated refresh token). Inference retries once after
            // a forced refresh; quota reads used to surface that 401 to the UI.
            Err(err) if is_auth_error(&err) => match self.refresh(true).await {
                Ok(()) => {
                    let creds = self.snapshot().await;
                    match fetch_usage(&creds).await {
                        Ok(snapshot) => snapshot,
                        Err(retry_err) if is_auth_error(&retry_err) => return Err(retry_err),
                        Err(retry_err) => return self.stale_or_error(cached, retry_err),
                    }
                }
                Err(refresh_err) if is_permanent_auth_error(&refresh_err) => {
                    return Err(refresh_err)
                }
                // A transient refresh failure is not "sign in again"; keep the
                // last snapshot when we have one and report a quota failure
                // otherwise.
                Err(refresh_err) => {
                    return self.stale_or_error(
                        cached,
                        refresh_err.context("ChatGPT token refresh failed"),
                    )
                }
            },
            // `/wham/usage` is optional; a network/5xx failure must not hide the
            // last snapshot captured from inference headers or SSE events.
            Err(err) => return self.stale_or_error(cached, err),
        };
        self.store_quota(snapshot.clone());
        Ok(Some(snapshot))
    }

    async fn fetch_quota_once(&self) -> anyhow::Result<QuotaSnapshot> {
        let creds = self.token(false).await?;
        fetch_usage(&creds).await
    }

    fn stale_or_error(
        &self,
        cached: Option<(u64, QuotaSnapshot)>,
        err: anyhow::Error,
    ) -> anyhow::Result<Option<QuotaSnapshot>> {
        match cached {
            Some((_, snapshot)) => Ok(Some(snapshot)),
            None => Err(err),
        }
    }

    pub fn observe_headers(&self, headers: &HeaderMap) {
        if let Some(snapshot) = quota_from_headers(headers) {
            self.store_quota(snapshot);
        }
    }

    pub fn observe_rate_limit(&self, value: &Value) {
        if let Some(mut snapshot) = quota_from_event(value) {
            // The event may omit `plan_type`; the credential's ID-token claim
            // is the same fallback the Docker edition uses.
            if snapshot.plan_type.is_empty() {
                if let Ok(creds) = self.credentials.try_read() {
                    snapshot.plan_type = creds.plan_type.clone();
                }
            }
            self.store_quota(snapshot);
        }
    }
}

/// Headers shared by inference, model discovery and quota reads. Keeping one
/// builder avoids the quota request silently losing a header that inference
/// gained (the original duplicated block was already missing user-agent).
pub(super) fn auth_headers(creds: &ChatGptCredentials, session: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", creds.access_token)) {
        headers.insert(AUTHORIZATION, value);
    }
    if !creds.account_id.is_empty() {
        if let Ok(value) = HeaderValue::from_str(&creds.account_id) {
            headers.insert("chatgpt-account-id", value);
        }
    }
    if let Ok(value) = HeaderValue::from_str(&originator()) {
        headers.insert("originator", value);
    }
    if let Ok(value) = HeaderValue::from_str(&format!("mix-agent/{}", env!("CARGO_PKG_VERSION"))) {
        headers.insert(reqwest::header::USER_AGENT, value);
    }
    if let Some(session) = session {
        if let Ok(value) = HeaderValue::from_str(session) {
            headers.insert("session-id", value.clone());
            headers.insert("thread-id", value);
        }
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    struct MemoryStore(StdMutex<Option<String>>);

    #[async_trait::async_trait]
    impl CredentialStore for MemoryStore {
        async fn load(&self) -> anyhow::Result<Option<String>> {
            Ok(self.0.lock().unwrap().clone())
        }

        async fn save(&self, raw: &str) -> anyhow::Result<()> {
            *self.0.lock().unwrap() = Some(raw.to_string());
            Ok(())
        }
    }

    fn creds(access: &str, refresh: &str, expires_at: u64) -> ChatGptCredentials {
        ChatGptCredentials {
            access_token: access.to_string(),
            refresh_token: refresh.to_string(),
            expires_at,
            ..Default::default()
        }
    }

    #[test]
    fn a_persisted_token_supersedes_even_when_it_needs_a_refresh() {
        let current = creds("old-access", "old-refresh", now_secs() + 3600);
        // Deliberately inside REFRESH_SKEW_SECONDS: the refresh token was
        // rotated, so adopting it is what avoids replaying the old one.
        let persisted = creds("new-access", "new-refresh", now_secs() + 60);
        assert!(persisted_supersedes(&current, &persisted));
    }

    #[test]
    fn an_identical_persisted_token_is_not_adopted() {
        let current = creds("same", "same", now_secs() + 3600);
        assert!(!persisted_supersedes(&current, &current));
    }

    #[tokio::test]
    async fn refresh_adopts_the_token_another_session_persisted() {
        let current = creds("old-access", "old-refresh", now_secs() + 3600);
        let persisted = creds("new-access", "new-refresh", now_secs() + 3600);
        let store = Arc::new(MemoryStore(StdMutex::new(Some(
            persisted.to_secret().unwrap(),
        ))));
        let raw = current.to_secret().unwrap();
        let session = ChatGptSession::new("provider-1", store, Some(&raw));

        // The persisted token is still valid, so no network call is made.
        session.refresh(false).await.expect("adopt persisted");
        let snapshot = session.snapshot().await;
        assert_eq!(snapshot.access_token, "new-access");
        assert_eq!(snapshot.refresh_token, "new-refresh");
    }

    #[tokio::test]
    async fn a_permanently_failed_login_fails_without_refreshing() {
        let raw = serde_json::to_string(&ChatGptCredentials {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: 0,
            auth_failed: true,
            ..Default::default()
        })
        .unwrap();
        let session = ChatGptSession::new("provider-1", Arc::new(NoopCredentialStore), Some(&raw));

        let err = session.token(false).await.unwrap_err();
        assert!(is_permanent_auth_error(&err), "{err}");
    }

    #[tokio::test]
    async fn marking_a_rejected_login_persists_auth_failed() {
        let store = Arc::new(MemoryStore(StdMutex::new(None)));
        let session = ChatGptSession::new(
            "provider-1",
            store.clone(),
            Some(&creds("a", "r", now_secs() + 3600).to_secret().unwrap()),
        );
        session.mark_auth_failed().await;
        assert!(session.snapshot().await.auth_failed);
        let saved = ChatGptCredentials::from_secret(store.0.lock().unwrap().as_deref());
        assert!(saved.auth_failed);
    }

    #[test]
    fn refresh_failures_are_classified() {
        use reqwest::StatusCode;
        assert!(classify_refresh_failure(StatusCode::UNAUTHORIZED, "{}").permanent);
        assert!(
            classify_refresh_failure(StatusCode::BAD_REQUEST, r#"{"error":"invalid_grant"}"#)
                .permanent
        );
        assert!(
            !classify_refresh_failure(StatusCode::BAD_REQUEST, r#"{"error":"proxy_noise"}"#)
                .permanent
        );
        assert!(!classify_refresh_failure(StatusCode::INTERNAL_SERVER_ERROR, "").permanent);
    }
}
