use crate::AppState;
use mixd_core::knowledge::{KnowledgeDoc, KnowledgeService};
use serde::Deserialize;
use tauri::State;

#[derive(serde::Serialize)]
pub struct KnowledgeDocView {
    pub doc_id: String,
    pub filename: String,
    pub chunk_count: usize,
    pub created_at: String,
}

impl From<KnowledgeDoc> for KnowledgeDocView {
    fn from(d: KnowledgeDoc) -> Self {
        Self {
            doc_id: d.doc_id,
            filename: d.filename,
            chunk_count: d.chunk_count,
            created_at: d.created_at.to_rfc3339(),
        }
    }
}

#[tauri::command]
pub async fn list_knowledge(state: State<'_, AppState>) -> Result<Vec<KnowledgeDocView>, String> {
    let docs = KnowledgeService::list(state.storage.as_ref())
        .await
        .map_err(|e| e.to_string())?;
    Ok(docs.into_iter().map(KnowledgeDocView::from).collect())
}

#[derive(Deserialize)]
pub struct KnowledgeIngest {
    pub filename: String,
    pub text: String,
}

#[tauri::command]
pub async fn ingest_knowledge(
    state: State<'_, AppState>,
    input: KnowledgeIngest,
) -> Result<KnowledgeDocView, String> {
    let doc = KnowledgeService::ingest(state.storage.as_ref(), &input.filename, &input.text)
        .await
        .map_err(|e| e.to_string())?;
    Ok(KnowledgeDocView::from(doc))
}

#[derive(Deserialize)]
pub struct KnowledgeImportPath {
    pub path: String,
}

#[tauri::command]
pub async fn import_knowledge_file(
    state: State<'_, AppState>,
    input: KnowledgeImportPath,
) -> Result<KnowledgeDocView, String> {
    let text = tokio::fs::read_to_string(&input.path)
        .await
        .map_err(|e| e.to_string())?;
    let filename = std::path::Path::new(&input.path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "document.txt".into());
    let doc = KnowledgeService::ingest(state.storage.as_ref(), &filename, &text)
        .await
        .map_err(|e| e.to_string())?;
    Ok(KnowledgeDocView::from(doc))
}

#[tauri::command]
pub async fn delete_knowledge(state: State<'_, AppState>, doc_id: String) -> Result<usize, String> {
    KnowledgeService::delete_doc(state.storage.as_ref(), &doc_id)
        .await
        .map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
pub struct KnowledgeHitView {
    pub id: String,
    pub doc_id: String,
    pub filename: String,
    pub text: String,
    pub score: f64,
}

#[tauri::command]
pub async fn search_knowledge(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<KnowledgeHitView>, String> {
    let hits = mixd_core::retrieval::retrieve_knowledge(
        state.storage.as_ref(),
        &query,
        limit.unwrap_or(10),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(hits
        .into_iter()
        .map(|h| KnowledgeHitView {
            id: h.id,
            doc_id: h.doc_id,
            filename: h.filename,
            text: h.text,
            score: h.score,
        })
        .collect())
}
