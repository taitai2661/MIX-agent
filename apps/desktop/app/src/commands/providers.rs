use crate::AppState;
use mixd_providers::preset as preset_catalog;
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Serialize)]
pub struct ProviderView {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub base_url: Option<String>,
    pub enabled: bool,
    pub has_api_key: bool,
    pub preset: Option<String>,
}

/// `(id, kind, label, base_url, enabled, preset)`
type ProviderRow = (String, String, String, Option<String>, i64, Option<String>);

#[tauri::command]
pub async fn list_providers(state: State<'_, AppState>) -> Result<Vec<ProviderView>, String> {
    let pool = state.storage.pool();
    let rows: Vec<ProviderRow> = sqlx::query_as(
        "SELECT id, json_extract(data, '$.kind') AS kind, json_extract(data, '$.label') AS label, \
         json_extract(data, '$.baseUrl') AS base_url, json_extract(data, '$.enabled') AS enabled, \
         json_extract(data, '$.preset') AS preset \
         FROM providers ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    let mut out = Vec::new();
    for (id, kind, label, base_url, enabled, preset) in rows {
        let key_purpose = format!("provider/{}", id);
        let has_api_key = state.secrets.get(&key_purpose, "api_key").await.is_ok();
        out.push(ProviderView {
            id,
            kind,
            label,
            base_url,
            enabled: enabled != 0,
            has_api_key,
            preset,
        });
    }
    Ok(out)
}

#[derive(Deserialize)]
pub struct ProviderCreate {
    pub preset_slug: Option<String>,
    pub kind: String,
    pub label: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub enabled: Option<bool>,
}

#[tauri::command]
pub async fn create_provider(
    state: State<'_, AppState>,
    input: ProviderCreate,
) -> Result<ProviderView, String> {
    let base_url = input.base_url.clone();
    let view = insert_provider(
        &state,
        input.preset_slug,
        &input.kind,
        &input.label,
        input.base_url.as_deref().unwrap_or_default(),
        input.api_key.as_deref(),
        input.enabled,
    )
    .await?;
    Ok(ProviderView { base_url, ..view })
}

/// Write a provider row and, when a key is given, its Keychain entry.
///
/// Shared by `create_provider` and the local-runtime import, so an imported
/// provider lands in the database exactly like a hand-entered one.
pub async fn insert_provider(
    state: &AppState,
    preset_slug: Option<String>,
    kind: &str,
    label: &str,
    base_url: &str,
    api_key: Option<&str>,
    enabled: Option<bool>,
) -> Result<ProviderView, String> {
    validate_provider_input(kind, label, base_url)?;
    // Labels identify providers in the UI; a duplicate silently splits models
    // across two rows with the same name.
    let duplicate: Option<String> =
        sqlx::query_scalar("SELECT id FROM providers WHERE json_extract(data, '$.label') = ?")
            .bind(label.trim())
            .fetch_optional(state.storage.pool())
            .await
            .map_err(crate::error::cmd_err)?;
    if duplicate.is_some() {
        return Err(format!(
            "a provider named `{}` already exists",
            label.trim()
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let base_url = base_url.trim().trim_end_matches('/');
    let enabled = enabled.unwrap_or(true);
    let data = serde_json::json!({
        "kind": kind,
        "label": label,
        "baseUrl": if base_url.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(base_url.to_string()) },
        "enabled": enabled,
        "preset": preset_slug,
        "extra": {},
    });
    sqlx::query("INSERT INTO providers (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .execute(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
    let has_api_key = match api_key {
        Some(key) if !key.is_empty() => {
            state
                .secrets
                .put(&format!("provider/{}", id), "api_key", key.as_bytes())
                .await
                .map_err(crate::error::cmd_err)?;
            true
        }
        _ => false,
    };
    // The Run Engine only knows about adapters in the live registry, so a
    // provider that is merely in the table is unusable until the next launch.
    // Registering here is what makes "add a provider, fetch its models, send a
    // message" work in one sitting instead of after a restart.
    crate::register_adapter(
        &state.engine.providers,
        state.secrets.as_ref(),
        &id,
        kind,
        base_url,
        enabled,
    )
    .await;
    // A new provider may become the embedding source (or the one that was
    // cached may no longer be the default's), so the answer is re-derived on
    // the next recall rather than at the next launch.
    state.embeddings.invalidate();
    Ok(ProviderView {
        id,
        kind: kind.to_string(),
        label: label.to_string(),
        base_url: (!base_url.is_empty()).then(|| base_url.to_string()),
        enabled,
        has_api_key,
        preset: preset_slug,
    })
}

/// Reject empty kinds/labels and malformed base URLs before a row is written.
fn validate_provider_input(kind: &str, label: &str, base_url: &str) -> Result<(), String> {
    if kind.trim().is_empty() {
        return Err("provider kind must not be empty".to_string());
    }
    if label.trim().is_empty() {
        return Err("provider label must not be empty".to_string());
    }
    if label.chars().count() > 100 {
        return Err("provider label too long (max 100 chars)".to_string());
    }
    let base_url = base_url.trim().trim_end_matches('/');
    if !base_url.is_empty() {
        let parsed: url::Url = base_url
            .parse()
            .map_err(|_| format!("invalid base_url: {base_url}"))?;
        match parsed.scheme() {
            "http" | "https" => {}
            _ => return Err(format!("invalid base_url scheme: {base_url}")),
        }
        if parsed.host_str().is_none() {
            return Err(format!("invalid base_url: {base_url}"));
        }
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct ProviderUpdate {
    pub id: String,
    pub label: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub enabled: Option<bool>,
}

#[tauri::command]
pub async fn update_provider(
    state: State<'_, AppState>,
    input: ProviderUpdate,
) -> Result<ProviderView, String> {
    let mut tx = state
        .storage
        .pool()
        .begin()
        .await
        .map_err(crate::error::cmd_err)?;
    let row: Option<(String,)> = sqlx::query_as("SELECT data FROM providers WHERE id = ?")
        .bind(&input.id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(crate::error::cmd_err)?;
    let mut data: serde_json::Value = match row {
        Some((existing,)) => {
            serde_json::from_str(&existing).unwrap_or_else(|_| serde_json::json!({}))
        }
        None => return Err(format!("provider not found: {}", input.id)),
    };
    if let Some(label) = &input.label {
        if label.trim().is_empty() {
            return Err("provider label must not be empty".to_string());
        }
        if label.chars().count() > 100 {
            return Err("provider label too long (max 100 chars)".to_string());
        }
        data["label"] = serde_json::Value::String(label.clone());
    }
    if let Some(url) = &input.base_url {
        validate_provider_input(
            data.get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("openai"),
            data.get("label")
                .and_then(|v| v.as_str())
                .unwrap_or("provider"),
            url,
        )?;
        data["baseUrl"] = serde_json::Value::String(url.clone());
    }
    if let Some(enabled) = input.enabled {
        data["enabled"] = serde_json::Value::Bool(enabled);
    }
    sqlx::query("UPDATE providers SET data = ? WHERE id = ?")
        .bind(data.to_string())
        .bind(&input.id)
        .execute(&mut *tx)
        .await
        .map_err(crate::error::cmd_err)?;
    tx.commit().await.map_err(crate::error::cmd_err)?;
    if let Some(key) = input.api_key.as_deref() {
        if !key.is_empty() {
            state
                .secrets
                .put(&format!("provider/{}", input.id), "api_key", key.as_bytes())
                .await
                .map_err(crate::error::cmd_err)?;
        }
    }
    let kind = data
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let base_url = data
        .get("baseUrl")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let enabled = data
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    // The adapter in the registry captured the old base URL and the old key at
    // construction time, so an edit here used to reach the database but not the
    // running engine: a corrected key kept failing with 401 until a restart.
    // Re-registering also covers `enabled`, where the adapter has to disappear
    // rather than merely be flagged.
    crate::register_adapter(
        &state.engine.providers,
        state.secrets.as_ref(),
        &input.id,
        &kind,
        &base_url,
        enabled,
    )
    .await;
    // The cached embedding source captured the old base URL / key / enabled
    // flag; drop it so the next recall rebuilds from the edited row.
    state.embeddings.invalidate();
    Ok(ProviderView {
        id: input.id.clone(),
        kind,
        label: data
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        base_url: (!base_url.is_empty()).then_some(base_url),
        enabled,
        has_api_key: state
            .secrets
            .get(&format!("provider/{}", input.id), "api_key")
            .await
            .is_ok(),
        preset: data
            .get("preset")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

#[tauri::command]
pub async fn delete_provider(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut tx = state
        .storage
        .pool()
        .begin()
        .await
        .map_err(crate::error::cmd_err)?;
    // Same transaction on purpose: a provider row without its models (or the
    // other way round) is exactly the orphaned state this deletes.
    super::models::delete_models_for_provider(&mut *tx, &id)
        .await
        .map_err(crate::error::cmd_err)?;
    sqlx::query("DELETE FROM providers WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(crate::error::cmd_err)?;
    tx.commit().await.map_err(crate::error::cmd_err)?;
    // Without this the adapter stayed reachable for the rest of the session:
    // `resolve_provider_id` matched the model's `provider:` prefix against the
    // registry, not against the table, so a deleted provider kept accepting
    // turns and failing them at the network layer.
    state.engine.providers.unregister(&id).await;
    if let Err(err) = state
        .secrets
        .delete(&format!("provider/{}", id), "api_key")
        .await
    {
        tracing::error!(provider = %id, %err, "removing the stored API key failed");
    }
    // The deleted row may have been the cached embedding source; the next
    // recall falls back to another provider or to lexical ranking.
    state.embeddings.invalidate();
    Ok(())
}

#[derive(Serialize)]
pub struct ProviderPresetView {
    pub slug: String,
    pub label: String,
    pub category: String,
    pub kind: String,
    pub default_url: String,
    pub api_key_required: bool,
    pub allow_private_default: bool,
    pub notes: String,
}

#[tauri::command]
pub async fn provider_presets() -> Result<Vec<ProviderPresetView>, String> {
    Ok(preset_catalog::PRESETS
        .iter()
        .map(|p| ProviderPresetView {
            slug: p.slug.to_string(),
            label: p.label.to_string(),
            category: p.category.to_string(),
            kind: p.kind.as_str().to_string(),
            default_url: p.default_url.to_string(),
            api_key_required: p.api_key_required,
            allow_private_default: p.allow_private_default,
            notes: p.notes.to_string(),
        })
        .collect())
}

/// Fetch the model list from a Provider and persist it. Equivalent to the
/// Docker edition's "モデル取得" action.
#[derive(serde::Deserialize)]
pub struct ConnectInput {
    pub id: String,
    pub discover: bool,
}

#[tauri::command]
pub async fn connect_provider(
    state: State<'_, AppState>,
    input: ConnectInput,
) -> Result<usize, String> {
    use mixd_providers::anthropic::ANTHROPIC_VERSION;
    use mixd_providers::discovery::join_api;
    use mixd_providers::preset::ProviderKind;

    /// How a provider expects the model-list call to be authenticated.
    enum ModelDiscoveryAuth {
        None,
        Bearer(String),
        Anthropic(String),
    }

    #[derive(Deserialize)]
    struct ModelList {
        #[serde(default)]
        data: Vec<ModelEntry>,
        #[serde(default)]
        models: Vec<ModelEntry>,
    }

    #[derive(Deserialize)]
    struct ModelEntry {
        #[serde(default)]
        id: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        context_length: Option<u32>,
        #[serde(default)]
        input_token_limit: Option<u32>,
    }

    let row: Option<(String, Option<String>, Option<i64>)> = sqlx::query_as(
        "SELECT json_extract(data, '$.kind') AS kind, json_extract(data, '$.baseUrl') AS base_url, \
         json_extract(data, '$.enabled') AS enabled \
         FROM providers WHERE id = ?",
    )
    .bind(&input.id)
    .fetch_optional(state.storage.pool())
    .await
    .map_err(crate::error::cmd_err)?;
    let (kind, base_url, enabled) = row.ok_or("provider not found")?;
    let enabled = enabled.unwrap_or(1) != 0;
    let base_url = base_url.ok_or("provider has no base url")?;
    let api_key = state
        .secrets
        .get_string(&format!("provider/{}", input.id), "api_key")
        .await
        .ok();

    // Model discovery differs per API family: the path *and* the auth header
    // are not interchangeable (Anthropic uses /v1/models + x-api-key, Gemini
    // takes the key as a query parameter, local runtimes need no auth at all).
    // `join_api` keeps a base URL that already ends in `/v1` from producing
    // `/v1/v1/models`, which is the form the local presets ship.
    let (url, auth) = match ProviderKind::parse(&kind) {
        Some(ProviderKind::Gemini) => (
            format!(
                "{}/v1beta/models?key={}",
                base_url.trim_end_matches('/'),
                api_key.clone().unwrap_or_default()
            ),
            ModelDiscoveryAuth::None,
        ),
        Some(ProviderKind::Anthropic) => (
            join_api(&base_url, "/v1/models"),
            match &api_key {
                Some(k) => ModelDiscoveryAuth::Anthropic(k.clone()),
                None => ModelDiscoveryAuth::None,
            },
        ),
        // Ollama and LM Studio both serve the OpenAI list unauthenticated.
        Some(ProviderKind::Ollama)
        | Some(ProviderKind::LMStudio)
        | Some(ProviderKind::Compatible) => {
            (join_api(&base_url, "/v1/models"), ModelDiscoveryAuth::None)
        }
        _ => (
            join_api(&base_url, "/models"),
            match &api_key {
                Some(k) => ModelDiscoveryAuth::Bearer(k.clone()),
                None => ModelDiscoveryAuth::None,
            },
        ),
    };

    // A dead endpoint has to fail fast: without a timeout a model-discovery
    // click against a stopped local server blocks on the OS connect timeout.
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_millis(1500))
        .timeout(std::time::Duration::from_secs(20))
        .no_proxy()
        .build()
        .map_err(crate::error::cmd_err)?;
    let mut req = client.get(&url);
    match &auth {
        ModelDiscoveryAuth::None => {}
        ModelDiscoveryAuth::Bearer(k) => req = req.bearer_auth(k),
        ModelDiscoveryAuth::Anthropic(k) => {
            req = req
                .header("x-api-key", k)
                .header("anthropic-version", ANTHROPIC_VERSION);
        }
    }
    let response = req
        .send()
        .await
        .map_err(|e| format!("model discovery failed: {}", e))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("model discovery returned {}", status));
    }
    let value: serde_json::Value = response.json().await.map_err(crate::error::cmd_err)?;
    let list: ModelList = serde_json::from_value(value).map_err(crate::error::cmd_err)?;
    let entries = if !list.data.is_empty() {
        list.data
    } else {
        list.models
    };
    if entries.is_empty() {
        return Err("provider returned no models".into());
    }

    let discovered: Vec<mixd_providers::discovery::DiscoveredModel> = entries
        .into_iter()
        .filter(|entry| !entry.id.is_empty())
        .map(|entry| mixd_providers::discovery::DiscoveredModel {
            label: entry.name.clone().unwrap_or_else(|| entry.id.clone()),
            id: entry.id.clone(),
            context_window: entry
                .context_length
                .or(entry.input_token_limit)
                .unwrap_or(0),
            parameter_size: None,
            quantization: None,
        })
        .collect();

    // Stamp the provider so a later listing can tell "never probed" from
    // "probed and answered".
    let _ =
        sqlx::query("UPDATE providers SET data = json_set(data, '$.status', 'ready') WHERE id = ?")
            .bind(&input.id)
            .execute(state.storage.pool())
            .await;
    let _ = input.discover;
    let written = super::models::sync_models(&state, &input.id, &discovered).await?;
    // Fetching a model list is the moment the user decides this provider is
    // ready, and it is also the first point at which the base URL is known to
    // be reachable. Registering here means the models just written are usable
    // on the next message, whether or not the provider predates this session.
    crate::register_adapter(
        &state.engine.providers,
        state.secrets.as_ref(),
        &input.id,
        &kind,
        &base_url,
        enabled,
    )
    .await;
    Ok(written.len())
}
