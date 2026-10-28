use crate::AppState;
use mixd_providers::Provider;
use serde::Serialize;
use std::sync::Arc;
use tauri::State;

#[derive(Serialize)]
pub struct ModelView {
    pub id: String,
    pub provider_id: String,
    pub label: String,
    pub context_window: u32,
    pub supports_vision: bool,
    pub supports_tool_calling: bool,
    pub supports_reasoning: bool,
    pub auto_candidate: bool,
    /// On-disk size from the runtime, when it publishes one.
    pub size_bytes: Option<u64>,
    pub parameter_size: Option<String>,
    pub quantization: Option<String>,
    pub family: Option<String>,
    /// Currently resident in the local runtime.
    pub loaded: bool,
    /// `native` / `probe` / `unknown` — where the capability flags came from.
    pub capability_source: Option<String>,
}

/// Persist a discovered model list for a provider, replacing any previous
/// entry for the same model.
///
/// Model ids are `<provider_id>:<model>` — the Run Engine splits on the first
/// colon to find the provider, so a model name containing colons (Ollama tags
/// like `llama3.1:8b`) stays intact.
pub async fn sync_models(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    provider_id: &str,
    models: &[mixd_providers::discovery::DiscoveredModel],
) -> Result<Vec<ModelView>, String> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut written = Vec::with_capacity(models.len());
    for model in models {
        if model.id.is_empty() {
            continue;
        }
        let model_id = format!("{provider_id}:{}", model.id);
        let capabilities = model.capabilities.as_ref();
        let data = serde_json::json!({
            "providerId": provider_id,
            "label": model.label,
            "contextWindow": model.context_window,
            // Native capabilities are authoritative when the runtime publishes
            // them. Otherwise the row starts unknown and `verify_tools` fills
            // it in with a real probe.
            "vision": capabilities.and_then(|caps| caps.vision).unwrap_or(false),
            "toolCalling": capabilities.and_then(|caps| caps.tools).unwrap_or(false),
            "reasoning": capabilities.and_then(|caps| caps.reasoning).unwrap_or(false),
            "auto": true,
            "sizeBytes": model.size_bytes,
            "parameterSize": model.parameter_size,
            "quantization": model.quantization,
            "family": model.family,
            "loaded": model.loaded,
            "capabilitySource": if capabilities.is_some() { "native" } else { "unknown" },
        });
        // Only the fields the runtime actually published are patched on a
        // re-sync. Replacing the row wholesale would reset the capability flags
        // that `verify_tools` wrote, and Composer would then refuse image
        // attachments or stop offering tools for a model that supports them.
        let mut discovered = serde_json::Map::new();
        discovered.insert("providerId".into(), provider_id.into());
        discovered.insert("label".into(), model.label.clone().into());
        if model.context_window > 0 {
            discovered.insert("contextWindow".into(), model.context_window.into());
        }
        if let Some(value) = model.size_bytes {
            discovered.insert("sizeBytes".into(), value.into());
        }
        if let Some(value) = &model.parameter_size {
            discovered.insert("parameterSize".into(), value.clone().into());
        }
        if let Some(value) = &model.quantization {
            discovered.insert("quantization".into(), value.clone().into());
        }
        if let Some(value) = &model.family {
            discovered.insert("family".into(), value.clone().into());
        }
        // `loaded` is a live state, not a capability: clearing it when a model
        // is unloaded is correct.
        discovered.insert("loaded".into(), model.loaded.into());
        if let Some(caps) = capabilities {
            for (key, value) in [
                ("vision", caps.vision),
                ("toolCalling", caps.tools),
                ("reasoning", caps.reasoning),
            ] {
                if let Some(value) = value {
                    discovered.insert(key.into(), value.into());
                }
            }
            discovered.insert("capabilitySource".into(), "native".into());
            discovered.insert("capabilitiesVerifiedAt".into(), now.clone().into());
        }
        sqlx::query(
            "INSERT INTO models (id, owner_id, data, created_at) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(id) DO UPDATE SET data = json_patch(data, json(?5))",
        )
        .bind(&model_id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .bind(&now)
        .bind(serde_json::Value::Object(discovered).to_string())
        .execute(pool)
        .await
        .map_err(crate::error::cmd_err)?;
        written.push(fetch_model_view(pool, &model_id).await?);
    }
    // A re-sync is an authoritative list for this provider. Without this pass a
    // model deleted from the runtime stayed in the DB forever, so the Composer
    // kept offering a tag that answered 404. The UI's "delete" action only
    // removes it from the runtime; the row is removed here.
    let keep: std::collections::HashSet<&str> =
        written.iter().map(|model| model.id.as_str()).collect();
    let existing: Vec<String> =
        sqlx::query_scalar("SELECT id FROM models WHERE json_extract(data, '$.providerId') = ?")
            .bind(provider_id)
            .fetch_all(pool)
            .await
            .map_err(crate::error::cmd_err)?;
    for id in existing {
        if keep.contains(id.as_str()) {
            continue;
        }
        sqlx::query("DELETE FROM models WHERE id = ?")
            .bind(&id)
            .execute(pool)
            .await
            .map_err(crate::error::cmd_err)?;
    }
    Ok(written)
}

/// Drop every model that belongs to `provider_id`.
///
/// Part of the provider deletion, in the same transaction as the provider row
/// itself. A model is only reachable through `data.$.providerId` — SQLite
/// enforces no foreign key between the two tables — so without this the rows
/// outlived their provider and kept showing up in the Composer. The predicate
/// matches the one `list_models` filters on, so a row cannot be hidden by one
/// query and deleted by the other.
///
/// Generic over the executor so it can join a caller's transaction.
pub async fn delete_models_for_provider<'e, E>(
    executor: E,
    provider_id: &str,
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query("DELETE FROM models WHERE json_extract(data, '$.providerId') = ?")
        .bind(provider_id)
        .execute(executor)
        .await
        .map(|_| ())
}

/// One model row, in the exact order [`MODEL_SELECT`] returns it.
///
/// `context_window` and `size_bytes` must be `i64`: `json_extract` returns JSON
/// numbers as SQLite `INTEGER`, so binding those columns to `String` makes the
/// whole query fail and the UI ends up rendering the empty state on a populated
/// table.
type ModelRow = (
    String,
    String,
    String,
    i64,
    i64,
    i64,
    i64,
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    Option<String>,
);

/// Shared projection so the list and single-row read cannot drift apart.
const MODEL_SELECT: &str = "SELECT id, \
     COALESCE(json_extract(data, '$.providerId'), '') AS provider_id, \
     COALESCE(json_extract(data, '$.label'), '') AS label, \
     COALESCE(json_extract(data, '$.contextWindow'), 0) AS context_window, \
     COALESCE(json_extract(data, '$.vision'), 0) AS vision, \
     COALESCE(json_extract(data, '$.toolCalling'), 0) AS tool_calling, \
     COALESCE(json_extract(data, '$.reasoning'), 0) AS reasoning, \
     COALESCE(json_extract(data, '$.auto'), 0) AS auto, \
     COALESCE(json_extract(data, '$.sizeBytes'), 0) AS size_bytes, \
     CAST(json_extract(data, '$.parameterSize') AS TEXT) AS parameter_size, \
     CAST(json_extract(data, '$.quantization') AS TEXT) AS quantization, \
     CAST(json_extract(data, '$.family') AS TEXT) AS family, \
     COALESCE(json_extract(data, '$.loaded'), 0) AS loaded, \
     CAST(json_extract(data, '$.capabilitySource') AS TEXT) AS capability_source \
     FROM models";

fn model_view_from_row(row: ModelRow) -> ModelView {
    let (
        id,
        provider_id,
        label,
        context_window,
        vision,
        tool_calling,
        reasoning,
        auto,
        size_bytes,
        parameter_size,
        quantization,
        family,
        loaded,
        capability_source,
    ) = row;
    let non_empty = |value: Option<String>| value.filter(|value| !value.is_empty());
    ModelView {
        id,
        provider_id,
        label,
        context_window: context_window.max(0) as u32,
        supports_vision: vision != 0,
        supports_tool_calling: tool_calling != 0,
        supports_reasoning: reasoning != 0,
        auto_candidate: auto != 0,
        size_bytes: size_bytes
            .filter(|value| *value > 0)
            .map(|value| value as u64),
        parameter_size: non_empty(parameter_size),
        quantization: non_empty(quantization),
        family: non_empty(family),
        loaded: loaded != 0,
        capability_source: non_empty(capability_source),
    }
}

/// Read one model row through the shared projection.
pub async fn fetch_model_view(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    model_id: &str,
) -> Result<ModelView, String> {
    let query = format!("{MODEL_SELECT} WHERE id = ?");
    let row: Option<ModelRow> = sqlx::query_as(&query)
        .bind(model_id)
        .fetch_optional(pool)
        .await
        .map_err(crate::error::cmd_err)?;
    row.map(model_view_from_row)
        .ok_or_else(|| "model not found".to_string())
}

/// Every model whose provider still exists, newest first.
///
/// Taken as a pool so it is reachable from a test as well as from the Tauri
/// command, matching how the other read paths in this crate are factored.
///
/// Models of a provider that no longer exists are filtered out here rather than
/// only at delete time: a row can also be left behind by an older version that
/// did not cascade, and the Composer would then happily offer a model that
/// cannot be served. `COALESCE` keeps a row with no `providerId` from making
/// `query_as` fail on a NULL — that failure takes the whole list down, not just
/// the one row, and an unrecognised row should cost one row.
pub async fn fetch_models(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<Vec<ModelView>, String> {
    let query = format!(
        "{MODEL_SELECT} \
         WHERE COALESCE(json_extract(data, '$.providerId'), '') IN (SELECT id FROM providers) \
         ORDER BY created_at DESC"
    );
    let rows: Vec<ModelRow> = sqlx::query_as(&query)
        .fetch_all(pool)
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(rows.into_iter().map(model_view_from_row).collect())
}

#[tauri::command]
pub async fn list_models(
    state: State<'_, AppState>,
    _provider_id: Option<String>,
) -> Result<Vec<ModelView>, String> {
    fetch_models(state.storage.pool()).await
}

#[tauri::command]
pub async fn verify_tools(
    state: State<'_, AppState>,
    model_id: String,
) -> Result<ModelView, String> {
    let pool = state.storage.pool();
    let row: Option<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT m.id, COALESCE(json_extract(m.data, '$.providerId'), ''), \
                json_extract(p.data, '$.kind'), json_extract(p.data, '$.baseUrl') \
         FROM models m LEFT JOIN providers p \
           ON p.id = json_extract(m.data, '$.providerId') \
         WHERE m.id = ?",
    )
    .bind(&model_id)
    .fetch_optional(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    let (id, provider_id, kind, base_url) = row.ok_or_else(|| "model not found".to_string())?;
    let provider: Arc<dyn Provider> = {
        let guard = state.engine.providers.inner.read().await;
        guard.get(&provider_id).cloned()
    }
    .ok_or_else(|| format!("provider not configured: {}", provider_id))?;
    // The stored id is `<provider_id>:<model>`; split on the first colon so an
    // Ollama tag (`llama3.1:8b`) keeps its tag.
    let model_name = id.split_once(':').map(|(_, name)| name).unwrap_or(&id);
    // A real probe is only safe where the transport is a local runtime: a
    // cloud endpoint that answers 401 or 429 must not be silently reclassified
    // as "no tools". For remote providers the static transport capability is
    // exactly the answer this command gave before.
    let is_local = match base_url.as_deref().filter(|url| !url.trim().is_empty()) {
        Some(url) => mixd_providers::discovery::is_local_base_url(url),
        None => matches!(kind.as_deref(), Some("ollama") | Some("lmstudio")),
    };
    let (supported, source) = if is_local {
        (
            mixd_providers::probe_tool_calling(provider.as_ref(), model_name)
                .await
                .map_err(crate::error::cmd_err)?,
            "probe",
        )
    } else {
        (provider.capabilities().tool_calling, "transport")
    };
    // Patch only the probed flag and its provenance. Vision/reasoning, the
    // context window and the Auto opt-in all survive a re-verification.
    sqlx::query(
        "UPDATE models SET data = json_set(data, \
          '$.toolCalling', ?, \
          '$.capabilitySource', ?, \
          '$.capabilitiesVerifiedAt', ?) \
         WHERE id = ?",
    )
    .bind(if supported { 1 } else { 0 })
    .bind(source)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(&model_id)
    .execute(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    fetch_model_view(pool, &model_id).await
}

/// Opt a model into or out of Auto routing.
///
/// Models discovered via `sync_models` start opted in; turning a model off
/// here removes it from future `auto` resolutions without deleting it.
#[tauri::command]
pub async fn set_model_auto(
    state: State<'_, AppState>,
    model_id: String,
    auto: bool,
) -> Result<ModelView, String> {
    let pool = state.storage.pool();
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM models WHERE id = ?")
        .bind(&model_id)
        .fetch_optional(pool)
        .await
        .map_err(crate::error::cmd_err)?;
    if exists.is_none() {
        return Err("model not found".to_string());
    }
    sqlx::query("UPDATE models SET data = json_set(data, '$.auto', ?) WHERE id = ?")
        .bind(if auto { 1 } else { 0 })
        .bind(&model_id)
        .execute(pool)
        .await
        .map_err(crate::error::cmd_err)?;
    fetch_model_view(pool, &model_id).await
}
