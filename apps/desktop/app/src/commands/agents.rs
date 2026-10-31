use crate::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Serialize)]
pub struct AgentView {
    pub id: String,
    pub name: String,
    pub instructions: String,
    pub tool_ids: Vec<String>,
    /// Optional `provider:model` override for this profile.
    #[serde(rename = "modelId")]
    pub model_id: Option<String>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[tauri::command]
pub async fn list_agents(state: State<'_, AppState>) -> Result<Vec<AgentView>, String> {
    let pool = state.storage.pool();
    let rows: Vec<(String, String, String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT id, json_extract(data, '$.name') AS name, json_extract(data, '$.instructions') AS instructions, \
         json_extract(data, '$.toolIds') AS tool_ids, \
         json_extract(data, '$.modelId') AS model_id, \
         COALESCE(json_extract(data, '$.updatedAt'), created_at) AS updated_at \
         FROM agents ORDER BY json_extract(data, '$.name')",
    )
    .fetch_all(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(rows
        .into_iter()
        .map(
            |(id, name, instructions, tool_ids, model_id, updated_at)| AgentView {
                id,
                name,
                instructions,
                tool_ids: serde_json::from_str::<Vec<String>>(&tool_ids).unwrap_or_default(),
                model_id: model_id.filter(|id| !id.trim().is_empty()),
                // `Utc::now()` here made every agent's relative time read "just
                // now", no matter when it was actually written.
                updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
            },
        )
        .collect())
}

#[derive(Deserialize)]
pub struct AgentCreate {
    pub name: String,
    pub instructions: String,
    pub tool_ids: Vec<String>,
    #[serde(rename = "modelId", default)]
    pub model_id: Option<String>,
}

#[tauri::command]
pub async fn create_agent(
    state: State<'_, AppState>,
    input: AgentCreate,
) -> Result<AgentView, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let data = serde_json::json!({
        "name": input.name,
        "instructions": input.instructions,
        "toolIds": input.tool_ids,
        "modelId": input.model_id.as_deref().map(str::trim).filter(|id| !id.is_empty()),
        "updatedAt": now.to_rfc3339(),
    });
    sqlx::query("INSERT INTO agents (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .execute(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
    state.engine.invalidate_catalogues().await;
    Ok(AgentView {
        id,
        name: input.name,
        instructions: input.instructions,
        tool_ids: input.tool_ids,
        model_id: input.model_id.filter(|id| !id.trim().is_empty()),
        updated_at: now,
    })
}

#[tauri::command]
pub async fn delete_agent(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let result = sqlx::query("DELETE FROM agents WHERE id = ?")
        .bind(&id)
        .execute(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
    if result.rows_affected() == 0 {
        return Err(format!("agent not found: {id}"));
    }
    state.engine.invalidate_catalogues().await;
    Ok(())
}

#[derive(Deserialize)]
pub struct AgentUpdate {
    pub id: String,
    pub name: String,
    pub instructions: String,
    pub tool_ids: Vec<String>,
    #[serde(rename = "modelId", default)]
    pub model_id: Option<String>,
}

#[tauri::command]
pub async fn update_agent(
    state: State<'_, AppState>,
    input: AgentUpdate,
) -> Result<AgentView, String> {
    let now = chrono::Utc::now();
    let data = serde_json::json!({
        "name": input.name,
        "instructions": input.instructions,
        "toolIds": input.tool_ids,
        "modelId": input.model_id.as_deref().map(str::trim).filter(|id| !id.is_empty()),
        "updatedAt": now.to_rfc3339(),
    });
    let result = sqlx::query("UPDATE agents SET data = ? WHERE id = ?")
        .bind(data.to_string())
        .bind(&input.id)
        .execute(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
    // Without this the command reported success for an id that no longer
    // exists, and the list was refreshed back to the same stale row.
    if result.rows_affected() == 0 {
        return Err(format!("agent not found: {}", input.id));
    }
    state.engine.invalidate_catalogues().await;
    Ok(AgentView {
        id: input.id,
        name: input.name,
        instructions: input.instructions,
        tool_ids: input.tool_ids,
        model_id: input.model_id.filter(|id| !id.trim().is_empty()),
        updated_at: now,
    })
}
