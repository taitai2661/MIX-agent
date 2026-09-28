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
    .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (id, kind, label, base_url, enabled, preset) in rows {
        let key_purpose = format!("provider/{}", id);
        let has_api_key = state.secrets.get(&key_purpose, "api_key").is_ok();
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
    let id = uuid::Uuid::new_v4().to_string();
    let base_url = input.base_url.clone();
    let data = serde_json::json!({
        "kind": input.kind,
        "label": input.label,
        "baseUrl": base_url,
        "enabled": input.enabled.unwrap_or(true),
        "preset": input.preset_slug,
        "extra": {},
    });
    sqlx::query("INSERT INTO providers (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(&data)
        .execute(state.storage.pool())
        .await
        .map_err(|e| e.to_string())?;
    if let Some(key) = input.api_key.as_deref() {
        if !key.is_empty() {
            state
                .secrets
                .put(&format!("provider/{}", id), "api_key", key.as_bytes())
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(ProviderView {
        id,
        kind: input.kind,
        label: input.label,
        base_url,
        enabled: input.enabled.unwrap_or(true),
        has_api_key: input
            .api_key
            .as_deref()
            .map(|k| !k.is_empty())
            .unwrap_or(false),
        preset: input.preset_slug,
    })
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
        .map_err(|e| e.to_string())?;
    let row: Option<(String,)> = sqlx::query_as("SELECT data FROM providers WHERE id = ?")
        .bind(&input.id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let mut data: serde_json::Value = match row {
        Some((existing,)) => {
            serde_json::from_str(&existing).unwrap_or_else(|_| serde_json::json!({}))
        }
        None => return Err(format!("provider not found: {}", input.id)),
    };
    if let Some(label) = &input.label {
        data["label"] = serde_json::Value::String(label.clone());
    }
    if let Some(url) = &input.base_url {
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
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    if let Some(key) = input.api_key.as_deref() {
        if !key.is_empty() {
            state
                .secrets
                .put(&format!("provider/{}", input.id), "api_key", key.as_bytes())
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(ProviderView {
        id: input.id.clone(),
        kind: data
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        label: data
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        base_url: data
            .get("baseUrl")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        enabled: data
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        has_api_key: state
            .secrets
            .get(&format!("provider/{}", input.id), "api_key")
            .is_ok(),
        preset: data
            .get("preset")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

#[tauri::command]
pub async fn delete_provider(state: State<'_, AppState>, id: String) -> Result<(), String> {
    sqlx::query("DELETE FROM providers WHERE id = ?")
        .bind(&id)
        .execute(state.storage.pool())
        .await
        .map_err(|e| e.to_string())?;
    let _ = state.secrets.delete(&format!("provider/{}", id), "api_key");
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

    let row: Option<(String, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT json_extract(data, '$.kind') AS kind, json_extract(data, '$.preset') AS preset, \
         json_extract(data, '$.baseUrl') AS base_url, json_extract(data, '$.enabled') AS enabled \
         FROM providers WHERE id = ?",
    )
    .bind(&input.id)
    .fetch_optional(state.storage.pool())
    .await
    .map_err(|e| e.to_string())?;
    let (kind, _preset, base_url, _enabled) = row.ok_or("provider not found")?;
    let base_url = base_url.ok_or("provider has no base url")?;
    let api_key = state
        .secrets
        .get_string(&format!("provider/{}", input.id), "api_key")
        .ok();

    // Model discovery differs per API family: the path *and* the auth header
    // are not interchangeable (Anthropic uses /v1/models + x-api-key, Gemini
    // takes the key as a query parameter, local runtimes need no auth at all).
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
            format!("{}/v1/models", base_url.trim_end_matches('/')),
            match &api_key {
                Some(k) => ModelDiscoveryAuth::Anthropic(k.clone()),
                None => ModelDiscoveryAuth::None,
            },
        ),
        Some(ProviderKind::Ollama) | Some(ProviderKind::LMStudio) => (
            format!("{}/v1/models", base_url.trim_end_matches('/')),
            ModelDiscoveryAuth::None,
        ),
        _ => (
            format!("{}/models", base_url.trim_end_matches('/')),
            match &api_key {
                Some(k) => ModelDiscoveryAuth::Bearer(k.clone()),
                None => ModelDiscoveryAuth::None,
            },
        ),
    };

    let client = reqwest::Client::new();
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
    let value: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let list: ModelList = serde_json::from_value(value).map_err(|e| e.to_string())?;
    let entries = if !list.data.is_empty() {
        list.data
    } else {
        list.models
    };
    if entries.is_empty() {
        return Err("provider returned no models".into());
    }

    let now = chrono::Utc::now();
    let mut written = 0usize;
    for entry in entries {
        if entry.id.is_empty() {
            continue;
        }
        let model_id = format!("{}:{}", input.id, entry.id);
        let data = serde_json::json!({
            "providerId": input.id,
            "label": entry.name.clone().unwrap_or_else(|| entry.id.clone()),
            "contextWindow": entry.context_length.or(entry.input_token_limit).unwrap_or(0),
            "vision": false,
            "toolCalling": false,
            "reasoning": false,
            "auto": true,
        });
        sqlx::query(
            "INSERT INTO models (id, owner_id, data, created_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET data = excluded.data",
        )
        .bind(&model_id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .bind(now.to_rfc3339())
        .execute(state.storage.pool())
        .await
        .map_err(|e| e.to_string())?;
        written += 1;
    }
    // Touch the provider so its status reflects a successful probe.
    let _ =
        sqlx::query("UPDATE providers SET data = json_set(data, '$.status', 'ready') WHERE id = ?")
            .bind(&input.id)
            .execute(state.storage.pool())
            .await;
    let _ = input.discover;
    Ok(written)
}
