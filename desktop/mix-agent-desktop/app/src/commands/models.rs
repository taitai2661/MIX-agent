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
        // Only the discovered fields are patched on conflict. Replacing the row
        // wholesale reset the three capability flags that `verify_tools` wrote,
        // so re-fetching the model list silently turned off Vision — and Composer
        // then refused image attachments.
        let discovered = serde_json::json!({
            "providerId": provider_id,
            "label": model.label,
            "contextWindow": model.context_window,
            "auto": true,
        });
        sqlx::query(
            "INSERT INTO models (id, owner_id, data, created_at) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(id) DO UPDATE SET data = json_patch(data, json(?5))",
        )
        .bind(&model_id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .bind(&now)
        .bind(discovered.to_string())
        .execute(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
        // Read the effective flags back: on a re-sync they are whatever the
        // stored row already had, not the `false` written above.
        let (vision, tool_calling, reasoning): (i64, i64, i64) = sqlx::query_as(
            "SELECT COALESCE(json_extract(data, '$.vision'), 0), \
                    COALESCE(json_extract(data, '$.toolCalling'), 0), \
                    COALESCE(json_extract(data, '$.reasoning'), 0) \
             FROM models WHERE id = ?",
        )
        .bind(&model_id)
        .fetch_one(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
        written.push(ModelView {
            id: model_id,
            provider_id: provider_id.to_string(),
            label: model.label.clone(),
            context_window: model.context_window,
            supports_vision: vision != 0,
            supports_tool_calling: tool_calling != 0,
            supports_reasoning: reasoning != 0,
        });
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

/// `(id, provider_id, label, context_window, vision, tool_calling, reasoning, auto)`
///
/// `context_window` must be `i64`: `json_extract` returns JSON numbers as
/// SQLite `INTEGER`, so binding the column to `String` makes the whole query
/// fail and the UI ends up rendering the empty state on a populated table.
type ModelRow = (String, String, String, i64, i64, i64, i64, i64);

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
    let rows: Vec<ModelRow> = sqlx::query_as(
        "SELECT id, COALESCE(json_extract(data, '$.providerId'), '') AS provider_id, \
         COALESCE(json_extract(data, '$.label'), '') AS label, \
         json_extract(data, '$.contextWindow') AS context_window, json_extract(data, '$.vision') AS vision, \
         json_extract(data, '$.toolCalling') AS tool_calling, json_extract(data, '$.reasoning') AS reasoning, \
         json_extract(data, '$.auto') AS auto \
         FROM models \
         WHERE COALESCE(json_extract(data, '$.providerId'), '') IN (SELECT id FROM providers) \
         ORDER BY created_at DESC",
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
