use crate::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Serialize)]
pub struct AgentView {
    pub id: String,
    pub name: String,
    pub instructions: String,
    pub tool_ids: Vec<String>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[tauri::command]
pub async fn list_agents(state: State<'_, AppState>) -> Result<Vec<AgentView>, String> {
    let pool = state.storage.pool();
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, json_extract(data, '$.name') AS name, json_extract(data, '$.instructions') AS instructions, \
         json_extract(data, '$.toolIds') AS tool_ids FROM agents ORDER BY json_extract(data, '$.name')",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|(id, name, instructions, tool_ids)| AgentView {
            id,
            name,
            instructions,
            tool_ids: serde_json::from_str::<Vec<String>>(&tool_ids).unwrap_or_default(),
            updated_at: chrono::Utc::now(),
        })
        .collect())
}

#[derive(Deserialize)]
pub struct AgentCreate {
    pub name: String,
    pub instructions: String,
    pub tool_ids: Vec<String>,
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
    });
    sqlx::query("INSERT INTO agents (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .execute(state.storage.pool())
        .await
        .map_err(|e| e.to_string())?;
    state.engine.invalidate_catalogues().await;
    Ok(AgentView {
        id,
        name: input.name,
        instructions: input.instructions,
        tool_ids: input.tool_ids,
        updated_at: now,
    })
}

#[tauri::command]
pub async fn delete_agent(state: State<'_, AppState>, id: String) -> Result<(), String> {
    sqlx::query("DELETE FROM agents WHERE id = ?")
        .bind(&id)
        .execute(state.storage.pool())
        .await
        .map_err(|e| e.to_string())?;
    state.engine.invalidate_catalogues().await;
    Ok(())
}

#[derive(Deserialize)]
pub struct AgentUpdate {
    pub id: String,
    pub name: String,
    pub instructions: String,
    pub tool_ids: Vec<String>,
}

#[tauri::command]
pub async fn update_agent(
    state: State<'_, AppState>,
    input: AgentUpdate,
) -> Result<AgentView, String> {
    let data = serde_json::json!({
        "name": input.name,
        "instructions": input.instructions,
        "toolIds": input.tool_ids,
    });
    sqlx::query("UPDATE agents SET data = ? WHERE id = ?")
        .bind(data.to_string())
        .bind(&input.id)
        .execute(state.storage.pool())
        .await
        .map_err(|e| e.to_string())?;
    state.engine.invalidate_catalogues().await;
    Ok(AgentView {
        id: input.id,
        name: input.name,
        instructions: input.instructions,
        tool_ids: input.tool_ids,
        updated_at: chrono::Utc::now(),
    })
}
