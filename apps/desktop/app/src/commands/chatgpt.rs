//! Tauri commands for the unofficial ChatGPT / Codex subscription provider.
//!
//! Device-code login is used on Desktop as well as Docker: it needs no
//! localhost callback and works behind proxies. The UI must show the
//! "unofficial, at your own risk" warning before calling these commands.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use mixd_providers::chatgpt::{self, is_auth_error, is_permanent_auth_error, ChatGptCredentials};
use mixd_providers::{Provider, QuotaSnapshot};
use serde::Serialize;
use tauri::State;

use crate::AppState;

const LOGIN_TTL_SECONDS: u64 = 15 * 60;

#[derive(Debug, Clone)]
struct DeviceLogin {
    provider_id: String,
    device_auth_id: String,
    user_code: String,
    interval: u64,
    next_poll_at: u64,
    expires_at: u64,
    /// Consecutive transient poll failures, used for bounded backoff.
    failures: u32,
}

fn logins() -> &'static Mutex<HashMap<String, DeviceLogin>> {
    static LOGINS: OnceLock<Mutex<HashMap<String, DeviceLogin>>> = OnceLock::new();
    LOGINS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Serialize)]
pub struct DeviceStartView {
    pub login_id: String,
    pub verification_url: String,
    pub user_code: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Serialize)]
pub struct DeviceStatusView {
    pub status: String,
    pub error: Option<String>,
    pub account: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct LogoutView {
    pub ok: bool,
}

#[derive(Serialize)]
pub struct ChatGptStatusView {
    /// Whether the unofficial integration is enabled in Settings.
    pub enabled: bool,
    /// The dedicated provider row, if it has been created.
    pub provider: Option<crate::commands::providers::ProviderView>,
    /// Models currently stored for that provider.
    pub models: u32,
}

/// Read the dedicated ChatGPT connection without creating anything.
///
/// The generic provider list also carries the row (models and the engine need
/// its id) but marks it `dedicated`, so the Providers tab hides it. This is the
/// only status endpoint the ChatGPT tab uses.
#[tauri::command]
pub async fn chatgpt_status(state: State<'_, AppState>) -> Result<ChatGptStatusView, String> {
    let settings = crate::commands::settings::read_settings(state.storage.pool()).await?;
    let provider = super::providers::provider_views(&state)
        .await?
        .into_iter()
        .find(|provider| provider.dedicated);
    let models = match &provider {
        Some(provider) => sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM models WHERE json_extract(data, '$.providerId') = ?",
        )
        .bind(&provider.id)
        .fetch_one(state.storage.pool())
        .await
        .unwrap_or(0)
        .max(0) as u32,
        None => 0,
    };
    Ok(ChatGptStatusView {
        enabled: settings.chatgpt_enabled,
        provider,
        models,
    })
}

/// Create the dedicated ChatGPT row on first use.
///
/// Kept separate from `create_provider` on purpose: the generic provider
/// command now rejects `chatgpt`, so a preset/tab mix-up cannot create a second
/// row or skip the fixed backend URL.
#[tauri::command]
pub async fn chatgpt_ensure(
    state: State<'_, AppState>,
) -> Result<crate::commands::providers::ProviderView, String> {
    let settings = crate::commands::settings::read_settings(state.storage.pool()).await?;
    if !settings.chatgpt_enabled {
        return Err(
            "the unofficial ChatGPT integration is disabled; enable it in Settings → ChatGPT"
                .to_string(),
        );
    }
    if let Some(provider) = super::providers::provider_views(&state)
        .await?
        .into_iter()
        .find(|provider| provider.dedicated)
    {
        return Ok(provider);
    }
    super::providers::insert_provider(
        &state,
        None,
        "chatgpt",
        "ChatGPT（Codex 使用枠）",
        chatgpt::BASE_URL,
        None,
        Some(true),
    )
    .await
}

fn pending_status() -> DeviceStatusView {
    DeviceStatusView {
        status: "pending".to_string(),
        error: None,
        account: None,
    }
}

fn expired_status(error: &str) -> DeviceStatusView {
    DeviceStatusView {
        status: "expired".to_string(),
        error: Some(error.to_string()),
        account: None,
    }
}

/// Encode an error the same way local-runtime failures are encoded, so the UI
/// can translate it instead of showing an internal code or English sentence.
fn local_error(code: &str, args: serde_json::Value) -> String {
    format!("mix-local-error:{code}|{args}")
}

fn quota_error(err: anyhow::Error) -> String {
    if is_permanent_auth_error(&err) {
        local_error("chatgpt_auth_required", serde_json::json!([]))
    } else if is_auth_error(&err) {
        local_error("chatgpt_auth_expired", serde_json::json!([]))
    } else {
        local_error("chatgpt_quota_failed", serde_json::json!([err.to_string()]))
    }
}

async fn provider_row(state: &AppState, id: &str) -> Result<(String, String, bool), String> {
    type Row = (Option<String>, Option<String>, Option<String>, Option<i64>);
    let row: Option<Row> = sqlx::query_as(
        "SELECT json_extract(data, '$.preset'), json_extract(data, '$.kind'), \
                    json_extract(data, '$.baseUrl'), json_extract(data, '$.enabled') \
             FROM providers WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(state.storage.pool())
    .await
    .map_err(crate::error::cmd_err)?;
    let (preset, kind, base_url, enabled) = row.ok_or_else(|| "provider not found".to_string())?;
    // Do not default a missing `kind` to "chatgpt": `is_chatgpt` is an OR of
    // preset and kind, so that default made every legacy row with a NULL kind
    // look like the unofficial integration even when its preset was OpenAI.
    let kind = kind.unwrap_or_default();
    if !chatgpt::is_chatgpt(preset.as_deref(), &kind) {
        return Err("this provider is not a ChatGPT integration".to_string());
    }
    Ok((
        kind,
        base_url.unwrap_or_else(|| chatgpt::BASE_URL.to_string()),
        enabled.unwrap_or(1) != 0,
    ))
}

async fn save_credentials(
    state: &AppState,
    provider_id: &str,
    creds: &ChatGptCredentials,
) -> Result<(), String> {
    let raw = creds.to_secret().map_err(crate::error::cmd_err)?;
    state
        .secrets
        .put(&format!("provider/{provider_id}"), "oauth", raw.as_bytes())
        .await
        .map_err(crate::error::cmd_err)?;
    let patch = serde_json::json!({
        "auth": creds.public_account(),
        "status": "ready",
        "billingMode": "subscription",
    });
    sqlx::query(
        "UPDATE providers SET data = json_set(data, '$.auth', json(?), \
         '$.status', 'ready', '$.billingMode', 'subscription') WHERE id = ?",
    )
    .bind(patch["auth"].to_string())
    .bind(provider_id)
    .execute(state.storage.pool())
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(())
}

async fn register(state: &AppState, provider_id: &str) {
    let Ok((_, base_url, enabled)) = provider_row(state, provider_id).await else {
        return;
    };
    crate::register_adapter(
        &state.engine.providers,
        state.secrets.as_ref(),
        provider_id,
        "chatgpt",
        Some("chatgpt"),
        &base_url,
        enabled,
    )
    .await;
}

#[tauri::command]
pub async fn chatgpt_device_start(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<DeviceStartView, String> {
    let _ = provider_row(&state, &provider_id).await?;
    let settings = crate::commands::settings::read_settings(state.storage.pool()).await?;
    if !settings.chatgpt_enabled {
        return Err("the unofficial ChatGPT integration is disabled".to_string());
    }
    let start = chatgpt::request_device_code().await.map_err(|err| {
        let message = err.to_string();
        let code = if message.to_ascii_lowercase().contains("not enabled") {
            "chatgpt_device_unavailable"
        } else {
            "chatgpt_device_failed"
        };
        local_error(code, serde_json::json!([message]))
    })?;
    let login_id = uuid::Uuid::new_v4().to_string();
    let now = now_secs();
    let entry = DeviceLogin {
        provider_id,
        device_auth_id: start.device_auth_id,
        user_code: start.user_code.clone(),
        interval: start.interval,
        next_poll_at: now,
        expires_at: now + LOGIN_TTL_SECONDS,
        failures: 0,
    };
    {
        let mut guard = logins()
            .lock()
            .map_err(|_| "login state is unavailable".to_string())?;
        guard.retain(|_, login| login.expires_at > now_secs());
        guard.insert(login_id.clone(), entry);
    }
    Ok(DeviceStartView {
        login_id,
        verification_url: start.verification_url,
        user_code: start.user_code,
        expires_in: LOGIN_TTL_SECONDS,
        interval: start.interval,
    })
}

#[tauri::command]
pub async fn chatgpt_device_poll(
    state: State<'_, AppState>,
    login_id: String,
) -> Result<DeviceStatusView, String> {
    let login = {
        let now = now_secs();
        let mut guard = logins()
            .lock()
            .map_err(|_| "login state is unavailable".to_string())?;
        let Some(entry) = guard.get(&login_id) else {
            return Ok(expired_status("chatgpt_login_not_found"));
        };
        if entry.expires_at <= now {
            guard.remove(&login_id);
            return Ok(expired_status("chatgpt_login_expired"));
        }
        // The Docker edition enforces the upstream `interval` server-side; a
        // misbehaving client (or a second window) must not hammer the auth
        // endpoint. The first poll is allowed immediately after start.
        if now < entry.next_poll_at {
            return Ok(pending_status());
        }
        let login = entry.clone();
        if let Some(entry) = guard.get_mut(&login_id) {
            entry.next_poll_at = now.saturating_add(entry.interval);
        }
        login
    };
    match chatgpt::poll_device_code(&login.device_auth_id, &login.user_code).await {
        Ok(Some(creds)) => {
            // Persist first, then always drop the in-memory flow. Leaving the
            // entry behind when the secret store rejected the write made the
            // UI offer a code that could never complete.
            let saved = save_credentials(&state, &login.provider_id, &creds).await;
            if let Ok(mut guard) = logins().lock() {
                guard.remove(&login_id);
            }
            saved?;
            register(&state, &login.provider_id).await;
            Ok(DeviceStatusView {
                status: "complete".to_string(),
                error: None,
                account: Some(creds.public_account()),
            })
        }
        Ok(None) => {
            if let Ok(mut guard) = logins().lock() {
                if let Some(entry) = guard.get_mut(&login_id) {
                    entry.failures = 0;
                    entry.next_poll_at = now_secs().saturating_add(entry.interval);
                }
            }
            Ok(pending_status())
        }
        Err(err) if !is_permanent_auth_error(&err) => {
            // A single timeout or 5xx must not throw away a code the user is
            // still typing. Keep the flow alive until the TTL and back off so
            // a broken network does not hammer the auth endpoint.
            if let Ok(mut guard) = logins().lock() {
                if let Some(entry) = guard.get_mut(&login_id) {
                    entry.failures = entry.failures.saturating_add(1);
                    let delay = (entry.interval.max(1) * (1u64 << entry.failures.min(4))).min(60);
                    entry.next_poll_at = now_secs().saturating_add(delay);
                }
            }
            tracing::warn!(%err, "ChatGPT device-code poll failed; retrying");
            Ok(pending_status())
        }
        Err(err) => {
            if let Ok(mut guard) = logins().lock() {
                guard.remove(&login_id);
            }
            let message = err.to_string();
            let code = if message.to_ascii_lowercase().contains("expired") {
                "chatgpt_login_expired"
            } else {
                "chatgpt_device_failed"
            };
            Ok(DeviceStatusView {
                status: "error".to_string(),
                error: Some(code.to_string()),
                account: None,
            })
        }
    }
}

#[tauri::command]
pub async fn chatgpt_device_cancel(login_id: String) -> Result<(), String> {
    let mut guard = logins()
        .lock()
        .map_err(|_| "login state is unavailable".to_string())?;
    guard.remove(&login_id);
    Ok(())
}

async fn resolve_provider(
    state: &AppState,
    provider_id: &str,
) -> Result<std::sync::Arc<dyn Provider>, String> {
    if let Some(provider) = state.engine.providers.get(provider_id).await {
        return Ok(provider);
    }
    let (_, base_url, enabled) = provider_row(state, provider_id).await?;
    crate::register_adapter(
        &state.engine.providers,
        state.secrets.as_ref(),
        provider_id,
        "chatgpt",
        Some("chatgpt"),
        &base_url,
        enabled,
    )
    .await;
    state
        .engine
        .providers
        .get(provider_id)
        .await
        .ok_or_else(|| "ChatGPT provider is not registered".to_string())
}

#[tauri::command]
pub async fn chatgpt_quota(
    state: State<'_, AppState>,
    provider_id: String,
    force: Option<bool>,
) -> Result<Option<QuotaSnapshot>, String> {
    let provider = resolve_provider(&state, &provider_id).await?;
    let result = if force.unwrap_or(false) {
        provider.refresh_quota().await
    } else {
        provider.quota().await
    };
    result.map_err(quota_error)
}

#[tauri::command]
pub async fn chatgpt_logout(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<LogoutView, String> {
    let _ = provider_row(&state, &provider_id).await?;
    let raw = state
        .secrets
        .get_string(&format!("provider/{provider_id}"), "oauth")
        .await
        .ok();
    if let Some(raw) = raw.as_deref() {
        let creds = ChatGptCredentials::from_secret(Some(raw));
        let _ = chatgpt::revoke_credentials(&creds).await;
    }
    let _ = state
        .secrets
        .delete(&format!("provider/{provider_id}"), "oauth")
        .await;
    sqlx::query(
        "UPDATE providers SET data = json_remove(json_set(data, \
         '$.status', 'needs_login'), '$.auth') WHERE id = ?",
    )
    .bind(&provider_id)
    .execute(state.storage.pool())
    .await
    .map_err(crate::error::cmd_err)?;
    state.engine.providers.unregister(&provider_id).await;
    // Stale models would stay selectable and fail with "login required".
    let _ = super::models::delete_models_for_provider(state.storage.pool(), &provider_id).await;
    Ok(LogoutView { ok: true })
}
