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
}

/// Persist a discovered model list for a provider, replacing any previous
/// entry for the same model.
///
/// Model ids are `<provider_id>:<model>` — the Run Engine splits on the first
/// colon to find the provider, so a model name containing colons (Ollama tags
/// like `llama3.1:8b`) stays intact.
pub async fn sync_models(
    state: &AppState,
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
        let data = serde_json::json!({
            "providerId": provider_id,
            "label": model.label,
            "contextWindow": model.context_window,
            // Unknown until `verify_tools` probes the live adapter. Guessing
            // here would advertise tool-calling for a model that lacks it.
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
        .bind(&now)
        .execute(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
        written.push(ModelView {
            id: model_id,
            provider_id: provider_id.to_string(),
            label: model.label.clone(),
            context_window: model.context_window,
            supports_vision: false,
            supports_tool_calling: false,
            supports_reasoning: false,
        });
    }
    Ok(written)
}

/// `(id, provider_id, label, context_window, vision, tool_calling, reasoning, auto)`
///
/// `context_window` must be `i64`: `json_extract` returns JSON numbers as
/// SQLite `INTEGER`, so binding the column to `String` makes the whole query
/// fail and the UI ends up rendering the empty state on a populated table.
type ModelRow = (String, String, String, i64, i64, i64, i64, i64);

#[tauri::command]
pub async fn list_models(
    state: State<'_, AppState>,
    _provider_id: Option<String>,
) -> Result<Vec<ModelView>, String> {
    let pool = state.storage.pool();
    let rows: Vec<ModelRow> = sqlx::query_as(
        "SELECT id, json_extract(data, '$.providerId') AS provider_id, json_extract(data, '$.label') AS label, \
         json_extract(data, '$.contextWindow') AS context_window, json_extract(data, '$.vision') AS vision, \
         json_extract(data, '$.toolCalling') AS tool_calling, json_extract(data, '$.reasoning') AS reasoning, \
         json_extract(data, '$.auto') AS auto \
         FROM models ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(rows
        .into_iter()
        .map(
            |(id, provider_id, label, context_window, vision, tool_calling, reasoning, _)| {
                ModelView {
                    id,
                    provider_id,
                    label,
                    context_window: context_window.max(0) as u32,
                    supports_vision: vision != 0,
                    supports_tool_calling: tool_calling != 0,
                    supports_reasoning: reasoning != 0,
                }
            },
        )
        .collect())
}

#[tauri::command]
pub async fn verify_tools(
    state: State<'_, AppState>,
    model_id: String,
) -> Result<ModelView, String> {
    let pool = state.storage.pool();
    let row: Option<(String, String, String, i64)> = sqlx::query_as(
        "SELECT id, json_extract(data, '$.providerId'), json_extract(data, '$.label'), json_extract(data, '$.contextWindow') \
         FROM models WHERE id = ?",
    )
    .bind(&model_id)
    .fetch_optional(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    let (id, provider_id, label, context_window) =
        row.ok_or_else(|| "model not found".to_string())?;
    let provider_key = provider_id.clone();
    let provider: Arc<dyn Provider> = {
        let guard = state.engine.providers.inner.read().await;
        guard.get(&provider_key).cloned()
    }
    .ok_or_else(|| format!("provider not configured: {}", provider_key))?;
    let caps = provider.capabilities();
    let data = serde_json::json!({
        "providerId": provider_id,
        "label": label,
        "contextWindow": context_window.max(0) as u32,
        "vision": caps.vision,
        "toolCalling": caps.tool_calling,
        "reasoning": caps.reasoning,
        "auto": false,
    });
    sqlx::query("UPDATE models SET data = ? WHERE id = ?")
        .bind(data.to_string())
        .bind(&model_id)
        .execute(pool)
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(ModelView {
        id,
        provider_id,
        label,
        context_window: context_window.max(0) as u32,
        supports_vision: caps.vision,
        supports_tool_calling: caps.tool_calling,
        supports_reasoning: caps.reasoning,
    })
}
