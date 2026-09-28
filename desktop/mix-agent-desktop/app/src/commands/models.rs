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

/// `(id, provider_id, label, context_window, vision, tool_calling, reasoning, auto)`
type ModelRow = (String, String, String, String, i64, i64, i64, i64);

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
    .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(
            |(id, provider_id, label, context_window, vision, tool_calling, reasoning, _)| {
                ModelView {
                    id,
                    provider_id,
                    label,
                    context_window: context_window.parse().unwrap_or(0),
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
    let row: Option<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, json_extract(data, '$.providerId'), json_extract(data, '$.label'), json_extract(data, '$.contextWindow') \
         FROM models WHERE id = ?",
    )
    .bind(&model_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
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
        "contextWindow": context_window.parse::<u32>().unwrap_or(0),
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
        .map_err(|e| e.to_string())?;
    Ok(ModelView {
        id,
        provider_id,
        label,
        context_window: context_window.parse().unwrap_or(0),
        supports_vision: caps.vision,
        supports_tool_calling: caps.tool_calling,
        supports_reasoning: caps.reasoning,
    })
}
