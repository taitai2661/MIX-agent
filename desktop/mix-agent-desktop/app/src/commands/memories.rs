use crate::AppState;
use mixd_core::memory::{MemoryCreate as CoreCreate, MemoryItem, MemoryService};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Serialize)]
pub struct MemoryView {
    pub id: String,
    pub title: String,
    pub body: String,
    pub role: String,
    pub scope: String,
    pub source_kind: String,
    pub lifecycle_state: String,
    pub verification: String,
    pub confidence: f64,
    pub salience: f64,
    pub strength: f64,
    pub updated_at: String,
}

impl From<MemoryItem> for MemoryView {
    fn from(m: MemoryItem) -> Self {
        Self {
            id: m.id,
            title: m.title,
            body: m.content,
            role: m.role,
            scope: m.scope,
            source_kind: m.source_kind,
            lifecycle_state: m.lifecycle_state,
            verification: m.verification,
            confidence: m.confidence,
            salience: m.salience,
            strength: m.strength,
            updated_at: m.updated_at.to_rfc3339(),
        }
    }
}

#[tauri::command]
pub async fn list_memories(state: State<'_, AppState>) -> Result<Vec<MemoryView>, String> {
    let items = MemoryService::list(state.storage.as_ref(), 200)
        .await
        .map_err(|e| e.to_string())?;
    Ok(items.into_iter().map(MemoryView::from).collect())
}

#[derive(Deserialize)]
pub struct MemoryCreate {
    pub title: String,
    pub body: String,
    pub role: Option<String>,
    pub scope: Option<String>,
    pub source_kind: Option<String>,
}

#[tauri::command]
pub async fn create_memory(
    state: State<'_, AppState>,
    input: MemoryCreate,
) -> Result<MemoryView, String> {
    let item = MemoryService::create(
        state.storage.as_ref(),
        CoreCreate {
            title: input.title,
            content: input.body,
            role: input.role.unwrap_or_else(|| "fact".into()),
            scope: input.scope.unwrap_or_else(|| "user".into()),
            source_kind: input.source_kind.unwrap_or_else(|| "user".into()),
            task_id: None,
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(MemoryView::from(item))
}

#[derive(Deserialize)]
pub struct MemoryUpdate {
    pub id: String,
    pub title: String,
    pub body: String,
}

#[tauri::command]
pub async fn update_memory(
    state: State<'_, AppState>,
    input: MemoryUpdate,
) -> Result<MemoryView, String> {
    let item = MemoryService::update(state.storage.as_ref(), &input.id, &input.title, &input.body)
        .await
        .map_err(|e| e.to_string())?;
    Ok(MemoryView::from(item))
}

#[tauri::command]
pub async fn delete_memory(state: State<'_, AppState>, id: String) -> Result<(), String> {
    MemoryService::delete(state.storage.as_ref(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct MemoryRevisionView {
    pub id: String,
    pub title: String,
    pub content: String,
    pub created_at: String,
}

#[tauri::command]
pub async fn list_memory_revisions(
    state: State<'_, AppState>,
    memory_id: String,
) -> Result<Vec<MemoryRevisionView>, String> {
    let revisions = MemoryService::revisions(state.storage.as_ref(), &memory_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(revisions
        .into_iter()
        .map(|r| MemoryRevisionView {
            id: r.id,
            title: r.title,
            content: r.content,
            created_at: r.created_at.to_rfc3339(),
        })
        .collect())
}

#[tauri::command]
pub async fn restore_memory_revision(
    state: State<'_, AppState>,
    memory_id: String,
    revision_id: String,
) -> Result<MemoryView, String> {
    let item = MemoryService::restore(state.storage.as_ref(), &memory_id, &revision_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(MemoryView::from(item))
}

#[tauri::command]
pub async fn verify_memory(
    state: State<'_, AppState>,
    id: String,
    verdict: String,
) -> Result<MemoryView, String> {
    MemoryService::verify(state.storage.as_ref(), &id, &verdict)
        .await
        .map_err(|e| e.to_string())?;
    let item = MemoryService::get(state.storage.as_ref(), &id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(MemoryView::from(item))
}

#[derive(Serialize)]
pub struct MemorySearchHit {
    pub id: String,
    pub title: String,
    pub content: String,
    pub role: String,
    pub scope: String,
    pub score: f64,
}

#[tauri::command]
pub async fn search_memories(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<MemorySearchHit>, String> {
    let hits = MemoryService::search(state.storage.as_ref(), &query, limit.unwrap_or(20))
        .await
        .map_err(|e| e.to_string())?;
    Ok(hits
        .into_iter()
        .map(|h| MemorySearchHit {
            id: h.id,
            title: h.title,
            content: h.content,
            role: h.role,
            scope: h.scope,
            score: h.score,
        })
        .collect())
}
